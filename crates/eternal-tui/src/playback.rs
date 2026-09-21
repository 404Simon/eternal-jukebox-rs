//! Playback controls shared by terminal input and MPRIS.

use std::time::{Duration, Instant};

use mpris_server::PlaybackStatus;
use rodio::Player;

pub(crate) const MAX_VOLUME: f32 = 2.0;

#[derive(Debug, Clone, Copy)]
pub(crate) enum Command {
    Play,
    Pause,
    Toggle,
    Stop,
    Volume(f32),
}

pub(crate) struct Playback {
    pub status: PlaybackStatus,
    pub volume: f32,
    clock: SessionClock,
}

impl Playback {
    pub fn new(volume: f32) -> Self {
        Self {
            status: PlaybackStatus::Playing,
            volume,
            clock: SessionClock::new(),
        }
    }

    pub fn elapsed(&self) -> Duration {
        self.clock.elapsed_at(Instant::now())
    }

    /// Returns true when the stream must be reset to its beginning.
    pub fn apply(&mut self, command: Command, player: &Player) -> bool {
        let status = match command {
            Command::Pause if self.status == PlaybackStatus::Stopped => return false,
            Command::Pause => PlaybackStatus::Paused,
            Command::Toggle if self.status == PlaybackStatus::Playing => PlaybackStatus::Paused,
            Command::Play | Command::Toggle => PlaybackStatus::Playing,
            Command::Stop => PlaybackStatus::Stopped,
            Command::Volume(volume) => {
                self.volume = volume.clamp(0.0, MAX_VOLUME);
                player.set_volume(self.volume);
                return false;
            }
        };
        if status == self.status {
            return false;
        }
        self.status = status;
        if status == PlaybackStatus::Playing {
            self.clock.resume();
            player.play();
        } else {
            player.pause();
            if status == PlaybackStatus::Stopped {
                self.clock = SessionClock::new();
                self.clock.paused_at = Some(self.clock.started);
            } else {
                self.clock.pause();
            }
        }
        status == PlaybackStatus::Stopped
    }
}

struct SessionClock {
    started: Instant,
    paused_at: Option<Instant>,
    paused_for: Duration,
}

impl SessionClock {
    fn new() -> Self {
        Self {
            started: Instant::now(),
            paused_at: None,
            paused_for: Duration::ZERO,
        }
    }

    fn pause(&mut self) {
        if self.paused_at.is_none() {
            self.paused_at = Some(Instant::now());
        }
    }

    fn resume(&mut self) {
        if let Some(paused_at) = self.paused_at.take() {
            self.paused_for += paused_at.elapsed();
        }
    }

    fn elapsed_at(&self, now: Instant) -> Duration {
        let current_pause = self
            .paused_at
            .map_or(Duration::ZERO, |paused_at| now.duration_since(paused_at));
        now.duration_since(self.started)
            .saturating_sub(self.paused_for + current_pause)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pause_and_play_are_idempotent_and_update_audio_and_clock() {
        let (player, _source) = Player::new();
        let mut playback = Playback::new(player.volume());
        playback.apply(Command::Pause, &player);
        let paused_at = playback.clock.paused_at;
        let elapsed = playback.elapsed();
        playback.apply(Command::Pause, &player);
        assert!(player.is_paused());
        assert_eq!(playback.status, PlaybackStatus::Paused);
        assert_eq!(playback.clock.paused_at, paused_at);
        assert_eq!(playback.elapsed(), elapsed);
        playback.apply(Command::Play, &player);
        let paused_for = playback.clock.paused_for;
        playback.apply(Command::Play, &player);
        assert!(!player.is_paused());
        assert_eq!(playback.status, PlaybackStatus::Playing);
        assert_eq!(playback.clock.paused_for, paused_for);
        playback.apply(Command::Toggle, &player);
        assert!(player.is_paused());
        playback.apply(Command::Toggle, &player);
        assert!(!player.is_paused());
    }

    #[test]
    fn stop_resets_clock_and_pause_does_not_resume_a_stopped_player() {
        let (player, _source) = Player::new();
        let mut playback = Playback::new(player.volume());
        assert!(playback.apply(Command::Stop, &player));
        assert_eq!(playback.elapsed(), Duration::ZERO);
        assert!(!playback.apply(Command::Stop, &player));
        playback.apply(Command::Pause, &player);
        assert_eq!(playback.status, PlaybackStatus::Stopped);
        assert!(player.is_paused());
        playback.apply(Command::Play, &player);
        assert!(!player.is_paused());
    }

    #[test]
    fn volume_changes_are_clamped_and_applied_to_audio() {
        let (player, _source) = Player::new();
        let mut playback = Playback::new(player.volume());
        for (requested, expected) in [(-1.0, 0.0), (0.4, 0.4), (3.0, MAX_VOLUME)] {
            playback.apply(Command::Volume(requested), &player);
            assert!((player.volume() - expected).abs() < f32::EPSILON);
            assert!((playback.volume - expected).abs() < f32::EPSILON);
        }
    }

    #[test]
    fn session_clock_does_not_advance_while_paused() {
        let started = Instant::now();
        let clock = SessionClock {
            started,
            paused_at: Some(started + Duration::from_secs(5)),
            paused_for: Duration::ZERO,
        };
        assert_eq!(
            clock.elapsed_at(started + Duration::from_secs(20)),
            Duration::from_secs(5)
        );
    }

    #[test]
    fn session_clock_excludes_completed_pauses() {
        let started = Instant::now();
        let clock = SessionClock {
            started,
            paused_at: None,
            paused_for: Duration::from_secs(7),
        };
        assert_eq!(
            clock.elapsed_at(started + Duration::from_secs(20)),
            Duration::from_secs(13)
        );
    }
}
