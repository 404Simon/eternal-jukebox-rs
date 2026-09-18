use std::{num::NonZero, thread, time::Duration};

use anyhow::{Context, Result};
use eternal_core::{Analysis, Audio, BranchGraph, PlaybackPlanner};
use rodio::{DeviceSinkBuilder, Player, buffer::SamplesBuffer};

const QUEUED_BEATS: usize = 8;
const FADE_MILLISECONDS: u32 = 3;

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
    let channels = NonZero::new(audio.channels).context("audio has no channels")?;
    let sample_rate = NonZero::new(audio.sample_rate).context("audio has no sample rate")?;
    let mut planner = match seed {
        Some(seed) => PlaybackPlanner::with_seed(graph, seed),
        None => PlaybackPlanner::new(graph),
    };

    loop {
        while player.len() < QUEUED_BEATS {
            let step = planner
                .next_step()
                .context("the playback graph contains no beats")?;
            let beat = &analysis.beats[step.beat];
            let samples = beat_samples(audio, beat.start, beat.duration);
            player.append(SamplesBuffer::new(channels, sample_rate, samples));
            if let Some(source) = step.jumped_from {
                eprintln!("jump {source} → {}", step.beat);
            }
        }
        thread::sleep(Duration::from_millis(25));
    }
}

fn beat_samples(audio: &Audio, start: f64, duration: f64) -> Vec<f32> {
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
            samples[frame * channels + channel] *= gain;
            let end = samples.len() - (frame + 1) * channels + channel;
            samples[end] *= gain;
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
            samples: vec![1.0; 200],
            sample_rate: 1_000,
            channels: 2,
        };
        let samples = beat_samples(&audio, 0.01, 0.05);
        assert_eq!(samples.len(), 100);
        assert!(samples[0].abs() < f32::EPSILON);
        assert!(samples[99].abs() < f32::EPSILON);
        assert!((samples[50] - 1.0).abs() < f32::EPSILON);
    }
}
