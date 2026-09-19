use std::f32::consts::TAU;

use realfft::RealFftPlanner;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::Audio;

const SILENCE_FLOOR_DB: f32 = -60.0;
const SILENCE_BELOW_PEAK_DB: f32 = 40.0;

#[derive(Clone, Debug)]
pub struct AnalysisConfig {
    pub frame_size: usize,
    pub hop_size: usize,
    pub minimum_bpm: f32,
    pub maximum_bpm: f32,
}

impl Default for AnalysisConfig {
    fn default() -> Self {
        Self {
            frame_size: 2_048,
            hop_size: 512,
            minimum_bpm: 70.0,
            maximum_bpm: 190.0,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Analysis {
    pub duration: f64,
    pub sample_rate: u32,
    pub channels: u16,
    pub tempo: f32,
    pub beats: Vec<Beat>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Beat {
    pub index: usize,
    pub start: f64,
    pub duration: f64,
    pub features: Features,
    /// Features near the attack, where a transition enters this beat.
    pub start_features: Features,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Features {
    /// Energy in each pitch class, starting at C.
    pub chroma: [f32; 12],
    /// Spectral centroid, bandwidth, rolloff and flatness, all normalised.
    pub timbre: [f32; 4],
    pub loudness_db: f32,
    pub onset_strength: f32,
}

#[derive(Debug, Error)]
pub enum AnalysisError {
    #[error("audio is too short; at least two seconds are required")]
    TooShort,
    #[error("invalid analysis configuration")]
    InvalidConfig,
    #[error("FFT failed: {0}")]
    Fft(String),
}

#[derive(Clone)]
struct Frame {
    time: f64,
    chroma: [f32; 12],
    spectral_energy: f32,
    weighted_frequency: f32,
    weighted_squared_frequency: f32,
    geometric_log_sum: f32,
    rolloff: f32,
    bins: usize,
    rms: f32,
    flux: f32,
}

/// Detect a beat grid and extract local musical features for every beat.
pub fn analyse(audio: &Audio, config: &AnalysisConfig) -> Result<Analysis, AnalysisError> {
    if audio.duration_seconds() < 2.0 {
        return Err(AnalysisError::TooShort);
    }
    if config.frame_size < 64
        || !config.frame_size.is_power_of_two()
        || config.hop_size == 0
        || config.minimum_bpm <= 0.0
        || config.maximum_bpm <= config.minimum_bpm
    {
        return Err(AnalysisError::InvalidConfig);
    }

    let frames = spectral_frames(audio, config)?;
    let onset = onset_envelope(&frames);
    let period = estimate_period(&onset, audio.sample_rate, config);
    let first = estimate_phase(&onset, period);
    let beat_frames = track_beat_positions(&onset, first, period);
    let beats = trim_silent_boundaries(describe_beats(&frames, &beat_frames, audio));
    let seconds_per_beat = period as f32 * config.hop_size as f32 / audio.sample_rate as f32;

    Ok(Analysis {
        duration: audio.duration_seconds(),
        sample_rate: audio.sample_rate,
        channels: audio.channels,
        tempo: 60.0 / seconds_per_beat,
        beats,
    })
}

fn spectral_frames(audio: &Audio, config: &AnalysisConfig) -> Result<Vec<Frame>, AnalysisError> {
    let mut planner = RealFftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(config.frame_size);
    let mut input = fft.make_input_vec();
    let mut spectrum = fft.make_output_vec();
    let mut previous = vec![0.0; spectrum.len()];
    let mut magnitudes = vec![0.0; spectrum.len()];
    let window: Vec<f32> = (0..config.frame_size)
        .map(|index| 0.5 - 0.5 * (TAU * index as f32 / config.frame_size as f32).cos())
        .collect();
    let mut frames = Vec::new();

    let channels = usize::from(audio.channels);
    let sample_rate = audio.sample_rate;
    let frame_count = audio.samples.len() / channels;
    for (frame_index, start) in (0..=frame_count.saturating_sub(config.frame_size))
        .step_by(config.hop_size)
        .enumerate()
    {
        let mut squared_samples = 0.0;
        for (offset, (output, weight)) in input.iter_mut().zip(&window).enumerate() {
            let sample = audio.samples
                [(start + offset) * channels..(start + offset + 1) * channels]
                .iter()
                .sum::<f32>()
                / channels as f32;
            *output = sample * weight;
            squared_samples += sample * sample;
        }
        fft.process(&mut input, &mut spectrum)
            .map_err(|error| AnalysisError::Fft(error.to_string()))?;
        for (magnitude, bin) in magnitudes.iter_mut().zip(&spectrum) {
            *magnitude = bin.norm();
        }
        let flux = magnitudes
            .iter()
            .zip(&previous)
            .map(|(current, old)| (current - old).max(0.0))
            .sum::<f32>()
            / magnitudes.len() as f32;
        let rms = (squared_samples / config.frame_size as f32).sqrt();
        let summary = summarise_spectrum(&magnitudes, sample_rate, config.frame_size);
        frames.push(Frame {
            time: (frame_index * config.hop_size) as f64 / f64::from(sample_rate),
            chroma: summary.chroma,
            spectral_energy: summary.spectral_energy,
            weighted_frequency: summary.weighted_frequency,
            weighted_squared_frequency: summary.weighted_squared_frequency,
            geometric_log_sum: summary.geometric_log_sum,
            rolloff: summary.rolloff,
            bins: summary.bins,
            rms,
            flux,
        });
        std::mem::swap(&mut previous, &mut magnitudes);
    }
    Ok(frames)
}

fn summarise_spectrum(magnitudes: &[f32], sample_rate: u32, fft_size: usize) -> Frame {
    let mut frame = Frame {
        time: 0.0,
        chroma: [0.0; 12],
        spectral_energy: magnitudes.iter().skip(1).sum(),
        weighted_frequency: 0.0,
        weighted_squared_frequency: 0.0,
        geometric_log_sum: 0.0,
        rolloff: 0.0,
        bins: magnitudes.len().saturating_sub(1),
        rms: 0.0,
        flux: 0.0,
    };
    let mut cumulative_energy = 0.0;
    for (bin, &magnitude) in magnitudes.iter().enumerate().skip(1) {
        let frequency = bin as f32 * sample_rate as f32 / fft_size as f32;
        if frequency >= 27.5 {
            let midi = 69.0 + 12.0 * (frequency / 440.0).log2();
            frame.chroma[(midi.round() as i32).rem_euclid(12) as usize] += magnitude;
        }
        frame.weighted_frequency += frequency * magnitude;
        frame.weighted_squared_frequency += frequency * frequency * magnitude;
        frame.geometric_log_sum += (magnitude + 1.0e-12).ln();
        cumulative_energy += magnitude;
        if frame.rolloff == 0.0 && cumulative_energy >= frame.spectral_energy * 0.85 {
            frame.rolloff = frequency;
        }
    }
    frame
}

fn onset_envelope(frames: &[Frame]) -> Vec<f32> {
    let mut values: Vec<f32> = frames.iter().map(|frame| frame.flux).collect();
    let raw = values.clone();
    for index in 0..values.len() {
        let start = index.saturating_sub(8);
        let mean = raw[start..=index].iter().sum::<f32>() / (index - start + 1) as f32;
        values[index] = (raw[index] - mean).max(0.0);
    }
    let peak = values.iter().copied().fold(0.0_f32, f32::max);
    if peak > 0.0 {
        for value in &mut values {
            *value /= peak;
        }
    }
    values
}

fn estimate_period(onset: &[f32], sample_rate: u32, config: &AnalysisConfig) -> usize {
    let frames_per_minute = 60.0 * sample_rate as f32 / config.hop_size as f32;
    let minimum_lag = (frames_per_minute / config.maximum_bpm).round() as usize;
    let maximum_lag = (frames_per_minute / config.minimum_bpm).round() as usize;
    (minimum_lag..=maximum_lag.min(onset.len().saturating_sub(1)))
        .max_by(|&left, &right| {
            autocorrelation(onset, left).total_cmp(&autocorrelation(onset, right))
        })
        .unwrap_or(minimum_lag.max(1))
}

fn autocorrelation(values: &[f32], lag: usize) -> f32 {
    values[..values.len() - lag]
        .iter()
        .zip(&values[lag..])
        .map(|(left, right)| left * right)
        .sum()
}

fn estimate_phase(onset: &[f32], period: usize) -> usize {
    (0..period.min(onset.len()))
        .max_by(|&left, &right| {
            phase_score(onset, left, period).total_cmp(&phase_score(onset, right, period))
        })
        .unwrap_or(0)
}

fn phase_score(onset: &[f32], phase: usize, period: usize) -> f32 {
    (phase..onset.len())
        .step_by(period)
        .map(|index| onset[index])
        .sum()
}

fn track_beat_positions(onset: &[f32], first: usize, period: usize) -> Vec<usize> {
    let period = period.max(1);
    let search_radius = (period / 8).max(1);
    let mut positions = Vec::new();
    let mut current = strongest_near(onset, first, search_radius);

    while current < onset.len() {
        positions.push(current);
        let predicted = current.saturating_add(period);
        if predicted >= onset.len() {
            break;
        }
        current = strongest_near(onset, predicted, search_radius);
        if current <= *positions.last().expect("a position was just pushed") {
            current = predicted;
        }
    }
    if positions
        .first()
        .is_some_and(|position| *position > period / 2)
    {
        positions.insert(0, 0);
    }
    positions
}

fn strongest_near(onset: &[f32], predicted: usize, radius: usize) -> usize {
    let start = predicted.saturating_sub(radius);
    let end = predicted.saturating_add(radius).min(onset.len() - 1);
    (start..=end)
        .max_by(|&left, &right| {
            onset_score(onset[left], left, predicted, radius).total_cmp(&onset_score(
                onset[right],
                right,
                predicted,
                radius,
            ))
        })
        .unwrap_or(predicted)
}

fn onset_score(strength: f32, position: usize, predicted: usize, radius: usize) -> f32 {
    let displacement = position.abs_diff(predicted) as f32 / radius as f32;
    strength - 0.35 * displacement.powi(2)
}

fn describe_beats(frames: &[Frame], positions: &[usize], audio: &Audio) -> Vec<Beat> {
    positions
        .iter()
        .enumerate()
        .map(|(index, &start)| {
            let end = positions.get(index + 1).copied().unwrap_or(frames.len());
            let frame_slice = &frames[start..end.max(start + 1).min(frames.len())];
            let start_seconds = frames[start].time;
            let end_seconds = positions
                .get(index + 1)
                .map_or(audio.duration_seconds(), |&next| frames[next].time);
            Beat {
                index,
                start: start_seconds,
                duration: end_seconds - start_seconds,
                features: aggregate_features(frame_slice, audio.sample_rate),
                start_features: aggregate_features(
                    &frame_slice[..(frame_slice.len() / 3).max(1)],
                    audio.sample_rate,
                ),
            }
        })
        .filter(|beat| beat.duration > 0.05)
        .collect()
}

fn trim_silent_boundaries(mut beats: Vec<Beat>) -> Vec<Beat> {
    let peak_loudness = beats
        .iter()
        .map(|beat| beat.features.loudness_db)
        .fold(f32::NEG_INFINITY, f32::max);
    let audible_threshold = (peak_loudness - SILENCE_BELOW_PEAK_DB).max(SILENCE_FLOOR_DB);
    let Some(first) = beats
        .iter()
        .position(|beat| beat.features.loudness_db > audible_threshold)
    else {
        return Vec::new();
    };
    let last = beats
        .iter()
        .rposition(|beat| beat.features.loudness_db > audible_threshold)
        .unwrap_or(first);

    let mut audible = beats.drain(first..=last).collect::<Vec<_>>();
    for (index, beat) in audible.iter_mut().enumerate() {
        beat.index = index;
    }
    audible
}

fn aggregate_features(frames: &[Frame], sample_rate: u32) -> Features {
    let mut chroma = [0.0; 12];
    let mut weighted_frequency = 0.0;
    let mut spectral_energy = 0.0;
    let mut squared_deviation = 0.0;
    let mut geometric_log_sum = 0.0;
    let mut rolloff_sum = 0.0;
    let mut bins = 0;
    let nyquist = sample_rate as f32 / 2.0;

    for frame in frames {
        for (total, value) in chroma.iter_mut().zip(frame.chroma) {
            *total += value;
        }
        weighted_frequency += frame.weighted_frequency;
        spectral_energy += frame.spectral_energy;
        squared_deviation += frame.weighted_squared_frequency;
        geometric_log_sum += frame.geometric_log_sum;
        rolloff_sum += frame.rolloff;
        bins += frame.bins;
    }
    let centroid = weighted_frequency / spectral_energy.max(1.0e-12);
    squared_deviation +=
        centroid * centroid * spectral_energy - 2.0 * centroid * weighted_frequency;
    squared_deviation = squared_deviation.max(0.0);
    let bandwidth = (squared_deviation / spectral_energy.max(1.0e-12)).sqrt();
    let arithmetic_mean = spectral_energy / bins.max(1) as f32;
    let flatness = (geometric_log_sum / bins.max(1) as f32).exp() / arithmetic_mean.max(1.0e-12);
    let chroma_sum = chroma.iter().sum::<f32>().max(1.0e-12);
    for value in &mut chroma {
        *value /= chroma_sum;
    }

    let average_rms = frames.iter().map(|frame| frame.rms).sum::<f32>() / frames.len() as f32;
    let onset_strength = frames.iter().map(|frame| frame.flux).sum::<f32>() / frames.len() as f32;
    Features {
        chroma,
        timbre: [
            centroid / nyquist,
            bandwidth / nyquist,
            rolloff_sum / frames.len() as f32 / nyquist,
            flatness,
        ],
        loudness_db: 20.0 * average_rms.max(1.0e-9).log10(),
        onset_strength,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn autocorrelation_finds_periodic_pulses() {
        let mut values = vec![0.0; 100];
        for index in (3..100).step_by(10) {
            values[index] = 1.0;
        }
        assert!(autocorrelation(&values, 10) > autocorrelation(&values, 9));
        assert_eq!(estimate_phase(&values, 10), 3);
    }

    #[test]
    fn beat_tracker_follows_local_timing_changes() {
        let mut onset = vec![0.0; 70];
        for position in [3, 14, 24, 35, 45, 56, 66] {
            onset[position] = 1.0;
        }
        assert_eq!(
            track_beat_positions(&onset, 3, 10),
            vec![3, 14, 24, 35, 45, 56, 66]
        );
    }

    #[test]
    fn silent_beats_are_trimmed_only_from_track_boundaries() {
        let loudness = [-180.0, -70.0, -12.0, -80.0, -18.0, -75.0];
        let beats = loudness
            .into_iter()
            .enumerate()
            .map(|(index, loudness_db)| Beat {
                index,
                start: index as f64,
                duration: 1.0,
                features: Features {
                    loudness_db,
                    ..Features::default()
                },
                start_features: Features::default(),
            })
            .collect();

        let trimmed = trim_silent_boundaries(beats);

        assert_eq!(trimmed.len(), 3);
        assert!((trimmed[0].start - 2.0).abs() < f64::EPSILON);
        assert!((trimmed[1].features.loudness_db - -80.0).abs() < f32::EPSILON);
        assert!((trimmed[2].start - 4.0).abs() < f64::EPSILON);
        assert_eq!(
            trimmed.iter().map(|beat| beat.index).collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
    }
}
