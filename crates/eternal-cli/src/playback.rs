use std::{num::NonZero, sync::Arc, thread, time::Duration};

use anyhow::{Context, Result};
use eternal_core::{Analysis, Audio, BranchGraph, PlaybackPlanner};
use rodio::{ChannelCount, DeviceSinkBuilder, Player, SampleRate, Source};

const QUEUED_BEATS: usize = 8;
const FADE_MILLISECONDS: u32 = 2;

pub fn play(
    audio: &Audio,
    analysis: &Analysis,
    graph: BranchGraph,
    seed: Option<u64>,
) -> Result<()> {
    let mut output = DeviceSinkBuilder::open_default_sink()
        .context("could not open the default audio output device")?;
    output.log_on_drop(false);
    let player = Player::connect_new(output.mixer());
    NonZero::new(audio.channels).context("audio has no channels")?;
    NonZero::new(audio.sample_rate).context("audio has no sample rate")?;
    let mut planner = match seed {
        Some(seed) => PlaybackPlanner::with_seed(graph, seed),
        None => PlaybackPlanner::new(graph),
    };
    let mut current = planner
        .next_step()
        .context("the playback graph contains no beats")?;

    loop {
        while player.len() < QUEUED_BEATS {
            let next = planner
                .next_step()
                .context("the playback graph contains no beats")?;
            let beat = &analysis.beats[current.beat];
            let source = BeatSource::new(
                audio,
                beat.start,
                beat.duration,
                current.jumped_from.is_some(),
                next.jumped_from.is_some(),
            );
            player.append(source);
            if let Some(source) = current.jumped_from {
                eprintln!("jump {source} → {}", current.beat);
            }
            current = next;
        }
        thread::sleep(Duration::from_millis(25));
    }
}

pub(crate) struct BeatSource {
    samples: Arc<[f32]>,
    start: usize,
    end: usize,
    position: usize,
    channels: ChannelCount,
    sample_rate: SampleRate,
    fade_frames: usize,
    fade_in: bool,
    fade_out: bool,
}

impl BeatSource {
    pub(crate) fn new(
        audio: &Audio,
        start: f64,
        duration: f64,
        fade_in: bool,
        fade_out: bool,
    ) -> Self {
        let channels = NonZero::new(audio.channels).expect("audio has channels");
        let sample_rate = NonZero::new(audio.sample_rate).expect("audio has a sample rate");
        let channel_count = usize::from(audio.channels);
        let frame_count = audio.samples.len() / channel_count;
        let start_frame = (start * f64::from(audio.sample_rate)).round() as usize;
        let end_frame = ((start + duration) * f64::from(audio.sample_rate)).round() as usize;
        let start_frame = start_frame.min(frame_count);
        let end_frame = end_frame.clamp(start_frame, frame_count);
        let fade_frames = ((audio.sample_rate * FADE_MILLISECONDS) / 1_000) as usize;
        let fade_frames = fade_frames.min((end_frame - start_frame) / 2);
        let start = start_frame * channel_count;
        let end = end_frame * channel_count;
        Self {
            samples: Arc::clone(&audio.samples),
            start,
            end,
            position: start,
            channels,
            sample_rate,
            fade_frames,
            fade_in,
            fade_out,
        }
    }
}

impl Iterator for BeatSource {
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        let mut sample = *self.samples.get(self.position)?;
        if self.position >= self.end {
            return None;
        }
        let channels = usize::from(self.channels.get());
        let frame = (self.position - self.start) / channels;
        let frame_count = (self.end - self.start) / channels;
        if self.fade_in && frame < self.fade_frames {
            sample *= frame as f32 / self.fade_frames.max(1) as f32;
        }
        if self.fade_out && frame >= frame_count - self.fade_frames {
            sample *= (frame_count - frame - 1) as f32 / self.fade_frames.max(1) as f32;
        }
        self.position += 1;
        Some(sample)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.end.saturating_sub(self.position);
        (remaining, Some(remaining))
    }
}

impl ExactSizeIterator for BeatSource {}

impl Source for BeatSource {
    fn current_span_len(&self) -> Option<usize> {
        Some(self.end.saturating_sub(self.position))
    }

    fn channels(&self) -> ChannelCount {
        self.channels
    }

    fn sample_rate(&self) -> SampleRate {
        self.sample_rate
    }

    fn total_duration(&self) -> Option<Duration> {
        let frames = (self.end - self.start) / usize::from(self.channels.get());
        Some(Duration::from_secs_f64(
            frames as f64 / f64::from(self.sample_rate.get()),
        ))
    }
}

#[cfg(test)]
fn beat_samples(
    audio: &Audio,
    start: f64,
    duration: f64,
    fade_in: bool,
    fade_out: bool,
) -> Vec<f32> {
    let channels = usize::from(audio.channels);
    let frame_count = audio.samples.len() / channels;
    let start_frame = (start * f64::from(audio.sample_rate)).round() as usize;
    let end_frame = ((start + duration) * f64::from(audio.sample_rate)).round() as usize;
    let start_frame = start_frame.min(frame_count);
    let end_frame = end_frame.clamp(start_frame, frame_count);
    let mut samples = audio.samples[start_frame * channels..end_frame * channels].to_vec();

    let fade_frames = ((audio.sample_rate * FADE_MILLISECONDS) / 1_000) as usize;
    let fade_frames = fade_frames.min((end_frame - start_frame) / 2);
    for frame in 0..fade_frames {
        let gain = frame as f32 / fade_frames.max(1) as f32;
        for channel in 0..channels {
            if fade_in {
                samples[frame * channels + channel] *= gain;
            }
            if fade_out {
                let end = samples.len() - (frame + 1) * channels + channel;
                samples[end] *= gain;
            }
        }
    }
    samples
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_interleaved_beat_and_fades_edges() {
        let audio = Audio {
            samples: vec![1.0; 200].into(),
            sample_rate: 1_000,
            channels: 2,
        };
        let samples = beat_samples(&audio, 0.01, 0.05, true, true);
        assert_eq!(samples.len(), 100);
        assert!(samples[0].abs() < f32::EPSILON);
        assert!(samples[99].abs() < f32::EPSILON);
        assert!((samples[50] - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn sequential_beats_are_not_faded() {
        let audio = Audio {
            samples: vec![1.0; 200].into(),
            sample_rate: 1_000,
            channels: 2,
        };
        let samples = beat_samples(&audio, 0.01, 0.05, false, false);
        assert!(
            samples
                .iter()
                .all(|sample| (*sample - 1.0).abs() < f32::EPSILON)
        );
    }

    #[test]
    fn shared_source_matches_materialised_samples() {
        let audio = Audio {
            samples: (0..400).map(|sample| sample as f32 / 400.0).collect(),
            sample_rate: 1_000,
            channels: 2,
        };
        for (fade_in, fade_out) in [(false, false), (true, false), (false, true), (true, true)] {
            let expected = beat_samples(&audio, 0.01, 0.05, fade_in, fade_out);
            let actual: Vec<_> = BeatSource::new(&audio, 0.01, 0.05, fade_in, fade_out).collect();
            assert_eq!(actual, expected);
        }
    }
}
