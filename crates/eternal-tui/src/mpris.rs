//! MPRIS transport. Only the TUI applies commands; D-Bus reads published snapshots.

use std::{
    future::Future,
    path::Path,
    sync::Mutex,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use async_channel::{Receiver, Sender};
use futures_lite::future::{block_on, race};
use mpris_server::{
    LoopStatus, Metadata, PlaybackRate, PlaybackStatus, PlayerInterface, Property, RootInterface,
    Server, Time, TrackId, Volume,
    zbus::{self, fdo},
};

use crate::ControlCommand;
use crate::playback::{Command, MAX_VOLUME};

const BUS_NAME: &str = "org.mpris.MediaPlayer2.eternal";
const OBJECT_PATH: &str = "/org/mpris/MediaPlayer2";
const PLAYER_INTERFACE: &str = "org.mpris.MediaPlayer2.Player";
const TIMEOUT: Duration = Duration::from_secs(3);
pub(crate) struct Request {
    pub command: Command,
    reply: Sender<()>,
}

impl Request {
    pub fn complete(self) {
        let _ = self.reply.try_send(());
    }
}

struct Snapshot {
    ready: bool,
    status: PlaybackStatus,
    volume: f32,
    position: Duration,
    updated: Instant,
}

struct Interface {
    commands: Sender<Request>,
    snapshot: Mutex<Snapshot>,
    metadata: Metadata,
}

impl Interface {
    async fn command(&self, command: Command) -> fdo::Result<()> {
        if !self.snapshot.lock().expect("snapshot poisoned").ready {
            return Err(fdo::Error::Failed("Player is still preparing".into()));
        }
        let (reply, received) = async_channel::bounded(1);
        self.commands
            .try_send(Request { command, reply })
            .map_err(|_| fdo::Error::Failed("Player is busy or shutting down".into()))?;
        received
            .recv()
            .await
            .map_err(|_| fdo::Error::Failed("Player shut down before applying the command".into()))
    }
}

pub(crate) struct Service {
    server: Server<Interface>,
    requests: Receiver<Request>,
}

impl Drop for Service {
    fn drop(&mut self) {
        self.requests.close();
        // Closing a channel retains queued values. Drain them so callers waiting
        // for acknowledgement receive an error when the TUI exits.
        while self.requests.try_recv().is_ok() {}
    }
}

impl Service {
    /// Claim the name before analysis, so another player cannot silently take control.
    /// A missing session bus must not prevent ordinary terminal playback.
    pub fn start(input: &Path) -> Result<Option<Self>> {
        match block_on(with_timeout(Self::new(input))) {
            Ok(service) => Ok(Some(service)),
            Err(zbus::Error::NameTaken) => bail!(
                "an Eternal session is already running; use `eternal play` or `eternal pause`"
            ),
            Err(error) => {
                eprintln!("Warning: MPRIS unavailable ({error}); external controls are disabled.");
                Ok(None)
            }
        }
    }

    async fn new(input: &Path) -> zbus::Result<Self> {
        let (commands, requests) = async_channel::bounded(16);
        let metadata = Metadata::builder()
            .trackid(TrackId::try_from("/org/mpris/MediaPlayer2/track/current")?)
            .title(
                input
                    .file_name()
                    .unwrap_or(input.as_os_str())
                    .to_string_lossy(),
            )
            // An endless generated stream has no finite mpris:length.
            .build();
        let server = Server::new(
            "eternal",
            Interface {
                commands,
                snapshot: Mutex::new(Snapshot {
                    ready: false,
                    status: PlaybackStatus::Stopped,
                    volume: 1.0,
                    position: Duration::ZERO,
                    updated: Instant::now(),
                }),
                metadata,
            },
        )
        .await?;
        // mpris-server initially permits name replacement. Remove that flag
        // before declaring startup successful. Use the bus directly because
        // Connection::request_name_with_flags caches already-owned names.
        let bus = fdo::DBusProxy::new(server.connection()).await?;
        match bus
            .request_name(
                BUS_NAME.try_into()?,
                fdo::RequestNameFlags::DoNotQueue.into(),
            )
            .await?
        {
            fdo::RequestNameReply::PrimaryOwner | fdo::RequestNameReply::AlreadyOwner => {}
            _ => return Err(zbus::Error::NameTaken),
        }
        Ok(Self { server, requests })
    }

    pub fn next_request(&self) -> Option<Request> {
        self.requests.try_recv().ok()
    }

    /// Publish after applying a command, before acknowledging it. Position updates
    /// are readable but do not emit `PropertiesChanged`, as required by MPRIS.
    #[allow(clippy::float_cmp)] // Notify for every actual change of the stored value.
    pub fn publish(&self, status: PlaybackStatus, volume: f32, position: Duration) -> Result<()> {
        let changes = {
            let mut snapshot = self
                .server
                .imp()
                .snapshot
                .lock()
                .expect("snapshot poisoned");
            let mut changes = Vec::new();
            if !snapshot.ready {
                changes.extend([Property::CanPlay(true), Property::CanPause(true)]);
            }
            if snapshot.status != status {
                changes.push(Property::PlaybackStatus(status));
            }
            if snapshot.volume != volume {
                changes.push(Property::Volume(f64::from(volume)));
            }
            *snapshot = Snapshot {
                ready: true,
                status,
                volume,
                position,
                updated: Instant::now(),
            };
            changes
        };
        if !changes.is_empty() {
            block_on(with_timeout(self.server.properties_changed(changes)))?;
        }
        Ok(())
    }
}

async fn with_timeout<T>(operation: impl Future<Output = zbus::Result<T>>) -> zbus::Result<T> {
    race(operation, async {
        async_io::Timer::after(TIMEOUT).await;
        Err(zbus::Error::Failure("MPRIS request timed out".into()))
    })
    .await
}

pub(crate) fn control(command: ControlCommand) -> Result<()> {
    block_on(with_timeout(async {
        let connection = zbus::connection::Builder::session()?
            .method_timeout(TIMEOUT)
            .build()
            .await?;
        let bus = fdo::DBusProxy::new(&connection).await?;
        if !bus.name_has_owner(BUS_NAME.try_into()?).await? {
            return Err(zbus::Error::Failure("No running Eternal session".into()));
        }
        let proxy = zbus::Proxy::new(&connection, BUS_NAME, OBJECT_PATH, PLAYER_INTERFACE).await?;
        match command {
            ControlCommand::Status => {
                let status: String = proxy.get_property("PlaybackStatus").await?;
                let ready: bool = proxy.get_property("CanPlay").await?;
                println!(
                    "{}",
                    if ready {
                        status.to_lowercase()
                    } else {
                        "starting".into()
                    }
                );
            }
            command => {
                let method = match command {
                    ControlCommand::Play => "Play",
                    ControlCommand::Pause => "Pause",
                    ControlCommand::Toggle => "PlayPause",
                    ControlCommand::Status => unreachable!(),
                };
                proxy.call::<_, _, ()>(method, &()).await?;
            }
        }
        Ok(())
    }))
    .context("could not control Eternal over the session D-Bus")
}

fn unsupported() -> fdo::Error {
    fdo::Error::NotSupported("This operation is not supported by Eternal".into())
}

impl RootInterface for Interface {
    async fn raise(&self) -> fdo::Result<()> {
        Err(unsupported())
    }
    async fn quit(&self) -> fdo::Result<()> {
        Err(unsupported())
    }
    async fn can_quit(&self) -> fdo::Result<bool> {
        Ok(false)
    }
    async fn fullscreen(&self) -> fdo::Result<bool> {
        Ok(false)
    }
    async fn set_fullscreen(&self, _: bool) -> zbus::Result<()> {
        Err(unsupported().into())
    }
    async fn can_set_fullscreen(&self) -> fdo::Result<bool> {
        Ok(false)
    }
    async fn can_raise(&self) -> fdo::Result<bool> {
        Ok(false)
    }
    async fn has_track_list(&self) -> fdo::Result<bool> {
        Ok(false)
    }
    async fn identity(&self) -> fdo::Result<String> {
        Ok("Eternal Jukebox".into())
    }
    async fn desktop_entry(&self) -> fdo::Result<String> {
        Ok(String::new())
    }
    async fn supported_uri_schemes(&self) -> fdo::Result<Vec<String>> {
        Ok(Vec::new())
    }
    async fn supported_mime_types(&self) -> fdo::Result<Vec<String>> {
        Ok(Vec::new())
    }
}

impl PlayerInterface for Interface {
    // The single endless stream has neither next/previous tracks nor absolute seeking.
    async fn next(&self) -> fdo::Result<()> {
        Ok(())
    }
    async fn previous(&self) -> fdo::Result<()> {
        Ok(())
    }
    async fn pause(&self) -> fdo::Result<()> {
        self.command(Command::Pause).await
    }
    async fn play_pause(&self) -> fdo::Result<()> {
        self.command(Command::Toggle).await
    }
    async fn stop(&self) -> fdo::Result<()> {
        self.command(Command::Stop).await
    }
    async fn play(&self) -> fdo::Result<()> {
        self.command(Command::Play).await
    }
    async fn seek(&self, _: Time) -> fdo::Result<()> {
        Ok(())
    }
    async fn set_position(&self, _: TrackId, _: Time) -> fdo::Result<()> {
        Ok(())
    }
    async fn open_uri(&self, _: String) -> fdo::Result<()> {
        Err(unsupported())
    }
    async fn playback_status(&self) -> fdo::Result<PlaybackStatus> {
        Ok(self.snapshot.lock().expect("snapshot poisoned").status)
    }
    async fn loop_status(&self) -> fdo::Result<LoopStatus> {
        Ok(LoopStatus::None)
    }
    async fn set_loop_status(&self, _: LoopStatus) -> zbus::Result<()> {
        Err(unsupported().into())
    }
    async fn rate(&self) -> fdo::Result<PlaybackRate> {
        Ok(1.0)
    }
    #[allow(clippy::float_cmp)] // Only exactly normal speed is supported.
    async fn set_rate(&self, rate: PlaybackRate) -> zbus::Result<()> {
        if rate == 1.0 {
            Ok(())
        } else if rate == 0.0 {
            self.command(Command::Pause).await.map_err(Into::into)
        } else {
            Err(unsupported().into())
        }
    }
    async fn shuffle(&self) -> fdo::Result<bool> {
        Ok(false)
    }
    async fn set_shuffle(&self, _: bool) -> zbus::Result<()> {
        Err(unsupported().into())
    }
    async fn metadata(&self) -> fdo::Result<Metadata> {
        Ok(self.metadata.clone())
    }
    async fn volume(&self) -> fdo::Result<Volume> {
        Ok(f64::from(
            self.snapshot.lock().expect("snapshot poisoned").volume,
        ))
    }
    async fn set_volume(&self, volume: Volume) -> zbus::Result<()> {
        if !volume.is_finite() {
            return Err(fdo::Error::InvalidArgs("Volume must be finite".into()).into());
        }
        self.command(Command::Volume(
            volume.clamp(0.0, f64::from(MAX_VOLUME)) as f32
        ))
        .await
        .map_err(Into::into)
    }
    async fn position(&self) -> fdo::Result<Time> {
        let snapshot = self.snapshot.lock().expect("snapshot poisoned");
        let elapsed = if snapshot.status == PlaybackStatus::Playing {
            snapshot.updated.elapsed()
        } else {
            Duration::ZERO
        };
        Ok(Time::from_micros(
            i64::try_from((snapshot.position + elapsed).as_micros()).unwrap_or(i64::MAX),
        ))
    }
    async fn minimum_rate(&self) -> fdo::Result<PlaybackRate> {
        Ok(1.0)
    }
    async fn maximum_rate(&self) -> fdo::Result<PlaybackRate> {
        Ok(1.0)
    }
    async fn can_go_next(&self) -> fdo::Result<bool> {
        Ok(false)
    }
    async fn can_go_previous(&self) -> fdo::Result<bool> {
        Ok(false)
    }
    async fn can_play(&self) -> fdo::Result<bool> {
        Ok(self.snapshot.lock().expect("snapshot poisoned").ready)
    }
    async fn can_pause(&self) -> fdo::Result<bool> {
        self.can_play().await
    }
    async fn can_seek(&self) -> fdo::Result<bool> {
        Ok(false)
    }
    async fn can_control(&self) -> fdo::Result<bool> {
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::playback::Playback;
    use futures_lite::StreamExt;
    use rodio::Player;

    // Run on a private bus: dbus-run-session -- cargo test -p eternal-tui
    // mpris_round_trip -- --ignored --nocapture
    #[test]
    #[ignore = "requires dbus-run-session and playerctl"]
    fn mpris_round_trip() {
        assert!(
            control(ControlCommand::Status)
                .unwrap_err()
                .to_string()
                .contains("D-Bus")
        );
        let service = block_on(Service::new(Path::new("test song.mp3"))).unwrap();
        assert!(matches!(
            block_on(Service::new(Path::new("second.mp3"))),
            Err(zbus::Error::NameTaken)
        ));
        assert!(
            format!("{:#}", control(ControlCommand::Play).unwrap_err()).contains("still preparing")
        );
        let (player, _source) = Player::new();
        let mut playback = Playback::new(player.volume());
        service
            .publish(playback.status, playback.volume, playback.elapsed())
            .unwrap();

        let client = std::thread::spawn(exercise_client);
        let deadline = Instant::now() + Duration::from_secs(20);
        while !client.is_finished() && Instant::now() < deadline {
            let request = block_on(race(async { service.requests.recv().await.ok() }, async {
                async_io::Timer::after(Duration::from_millis(10)).await;
                None
            }));
            if let Some(request) = request {
                playback.apply(request.command, &player);
                assert_eq!(
                    player.is_paused(),
                    playback.status != PlaybackStatus::Playing
                );
                service
                    .publish(playback.status, playback.volume, playback.elapsed())
                    .unwrap();
                request.complete();
            }
        }
        assert!(client.is_finished(), "MPRIS client hung");
        client.join().unwrap();
        let (reply, received) = async_channel::bounded(1);
        assert!(
            service
                .server
                .imp()
                .commands
                .try_send(Request {
                    command: Command::Pause,
                    reply
                })
                .is_ok()
        );
        drop(service);
        assert!(matches!(
            received.try_recv(),
            Err(async_channel::TryRecvError::Closed)
        ));
        assert!(control(ControlCommand::Pause).is_err());
        // A clean shutdown releases the name; no stale socket or PID file remains.
        let _replacement = block_on(Service::new(Path::new("replacement.mp3"))).unwrap();
    }

    async fn assert_status(proxy: &zbus::Proxy<'_>, expected: &str) {
        assert_eq!(
            proxy
                .get_property::<String>("PlaybackStatus")
                .await
                .unwrap(),
            expected
        );
    }

    fn exercise_client() {
        block_on(async {
            let connection = zbus::connection::Builder::session()
                .unwrap()
                .method_timeout(TIMEOUT)
                .build()
                .await
                .unwrap();
            let proxy = zbus::proxy::Builder::<zbus::Proxy<'_>>::new(&connection)
                .destination(BUS_NAME)
                .unwrap()
                .path(OBJECT_PATH)
                .unwrap()
                .interface(PLAYER_INTERFACE)
                .unwrap()
                .cache_properties(zbus::proxy::CacheProperties::No)
                .build()
                .await
                .unwrap();
            let properties = fdo::PropertiesProxy::builder(&connection)
                .destination(BUS_NAME)
                .unwrap()
                .path(OBJECT_PATH)
                .unwrap()
                .build()
                .await
                .unwrap();
            let mut signals = properties.receive_properties_changed().await.unwrap();

            control(ControlCommand::Pause).unwrap();
            let signal = race(async { signals.next().await }, async {
                async_io::Timer::after(TIMEOUT).await;
                None
            })
            .await
            .expect("playback change must emit a signal");
            let args = signal.args().unwrap();
            assert_eq!(args.interface_name().as_str(), PLAYER_INTERFACE);
            assert_eq!(
                args.changed_properties()
                    .get("PlaybackStatus")
                    .unwrap()
                    .downcast_ref::<&str>()
                    .unwrap(),
                "Paused"
            );
            assert_status(&proxy, "Paused").await;
            let position: i64 = proxy.get_property("Position").await.unwrap();
            control(ControlCommand::Pause).unwrap();
            assert_eq!(
                proxy.get_property::<i64>("Position").await.unwrap(),
                position
            );
            control(ControlCommand::Play).unwrap();
            control(ControlCommand::Play).unwrap();
            assert_status(&proxy, "Playing").await;
            control(ControlCommand::Toggle).unwrap();
            assert_status(&proxy, "Paused").await;

            proxy.set_property("Volume", 0.4_f64).await.unwrap();
            assert!((proxy.get_property::<f64>("Volume").await.unwrap() - 0.4).abs() < 1e-6);
            proxy.set_property("Volume", 4.0_f64).await.unwrap();
            assert!(
                (proxy.get_property::<f64>("Volume").await.unwrap() - 2.0).abs() < f64::EPSILON
            );
            assert!(proxy.set_property("Volume", f64::NAN).await.is_err());
            assert!(!proxy.get_property::<bool>("CanSeek").await.unwrap());
            let metadata: std::collections::HashMap<String, zbus::zvariant::OwnedValue> =
                proxy.get_property("Metadata").await.unwrap();
            assert_eq!(
                metadata["xesam:title"].downcast_ref::<&str>().unwrap(),
                "test song.mp3"
            );

            proxy.call::<_, _, ()>("Stop", &()).await.unwrap();
            assert_status(&proxy, "Stopped").await;
            assert_eq!(proxy.get_property::<i64>("Position").await.unwrap(), 0);
            control(ControlCommand::Play).unwrap();
            assert_status(&proxy, "Playing").await;
            exercise_playerctl();
        });
    }

    fn exercise_playerctl() {
        fn run(args: &[&str]) -> String {
            let output = std::process::Command::new("playerctl")
                .args(["--player=eternal"])
                .args(args)
                .output()
                .expect("install playerctl to run the MPRIS integration test");
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            String::from_utf8(output.stdout).unwrap().trim().to_owned()
        }
        assert_eq!(run(&["status"]), "Playing");
        run(&["pause"]);
        assert_eq!(run(&["status"]), "Paused");
        run(&["play-pause"]);
        assert_eq!(run(&["status"]), "Playing");
        assert_eq!(run(&["metadata", "xesam:title"]), "test song.mp3");
        run(&["volume", "0.5"]);
        assert!((run(&["volume"]).parse::<f64>().unwrap() - 0.5).abs() < f64::EPSILON);
    }
}
