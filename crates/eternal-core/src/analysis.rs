use std::f32::consts::TAU;

use realfft::RealFftPlanner;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::Audio;

const SILENCE_FLOOR_DB: f32 = -60.0;
const SILENCE_BELOW_PEAK_DB: f32 = 40.0;
const SPECTRAL_BAND_EDGES: [f32; 5] = [80.0, 200.0, 800.0, 2_000.0, 6_000.0];
const SPECTRAL_BAND_COUNT: usize = SPECTRAL_BAND_EDGES.len() + 1;
const ONSET_PROFILE_SIZE: usize = 4;
const PERIOD_MULTIPLES: [usize; 5] = [1, 2, 4, 8, 16];

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
    /// Relative energy from sub-bass through high frequencies.
    #[serde(default)]
    pub spectral_bands: [f32; SPECTRAL_BAND_COUNT],
    pub loudness_db: f32,
    pub onset_strength: f32,
    /// Transient energy over four equally sized parts of the beat.
    #[serde(default)]
    pub onset_profile: [f32; ONSET_PROFILE_SIZE],
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
    spectral_bands: [f32; SPECTRAL_BAND_COUNT],
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
    let coarse_period = estimate_period(&onset, audio.sample_rate, config);
    let approximate_period = refine_period(&onset, coarse_period);
    let (period, first) = refine_grid(&onset, approximate_period);
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
    let mut mono = vec![0.0; config.frame_size];
    for (frame_index, start) in (0..=frame_count.saturating_sub(config.frame_size))
        .step_by(config.hop_size)
        .enumerate()
    {
        if frame_index == 0 || config.hop_size >= config.frame_size {
            downmix_into(audio, start, &mut mono);
        } else {
            let retained = config.frame_size - config.hop_size;
            mono.copy_within(config.hop_size.., 0);
            downmix_into(
                audio,
                start + retained,
                &mut mono[retained..config.frame_size],
            );
        }
        let mut squared_samples = 0.0;
        for ((output, &sample), weight) in input.iter_mut().zip(&mono).zip(&window) {
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
            spectral_bands: summary.spectral_bands,
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

fn downmix_into(audio: &Audio, start_frame: usize, output: &mut [f32]) {
    let channels = usize::from(audio.channels);
    for (offset, sample) in output.iter_mut().enumerate() {
        *sample = audio.samples
            [(start_frame + offset) * channels..(start_frame + offset + 1) * channels]
            .iter()
            .sum::<f32>()
            / channels as f32;
    }
}

fn summarise_spectrum(magnitudes: &[f32], sample_rate: u32, fft_size: usize) -> Frame {
    let mut frame = Frame {
        time: 0.0,
        chroma: [0.0; 12],
        spectral_bands: [0.0; SPECTRAL_BAND_COUNT],
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
        let band = spectral_band(frequency);
        frame.spectral_bands[band] += magnitude;
        frame.weighted_frequency += frequency * magnitude;
        frame.weighted_squared_frequency += frequency * frequency * magnitude;
        frame.geometric_log_sum += (magnitude + 1.0e-12).ln();
        cumulative_energy += magnitude;
        if frame.rolloff == 0.0 && cumulative_energy >= frame.spectral_energy * 0.85 {
            frame.rolloff = frequency;
        }
    }
    for energy in &mut frame.spectral_bands {
        *energy /= frame.spectral_energy.max(1.0e-12);
    }
    frame
}

fn spectral_band(frequency: f32) -> usize {
    SPECTRAL_BAND_EDGES.partition_point(|edge| frequency >= *edge)
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
    let selected = (minimum_lag..=maximum_lag.min(onset.len().saturating_sub(1)))
        .max_by(|&left, &right| {
            autocorrelation(onset, left).total_cmp(&autocorrelation(onset, right))
        })
        .unwrap_or(minimum_lag.max(1));
    prefer_supported_faster_pulse(onset, selected, minimum_lag)
}

fn prefer_supported_faster_pulse(onset: &[f32], mut period: usize, minimum_lag: usize) -> usize {
    // Alternating strong and weak beats often make the two-beat period score
    // slightly higher than the actual pulse. Prefer the faster octave when it
    // retains most of the autocorrelation support instead of returning a
    // half-tempo beat grid.
    loop {
        let half = period / 2;
        if half < minimum_lag {
            break;
        }
        let faster = half.saturating_sub(1)..=half.saturating_add(1);
        let faster = faster
            .filter(|lag| *lag >= minimum_lag && *lag < onset.len())
            .max_by(|&left, &right| {
                autocorrelation(onset, left).total_cmp(&autocorrelation(onset, right))
            })
            .unwrap_or(half);
        if autocorrelation(onset, faster) < 0.8 * autocorrelation(onset, period) {
            break;
        }
        period = faster;
    }
    period
}

fn autocorrelation(values: &[f32], lag: usize) -> f32 {
    values[..values.len() - lag]
        .iter()
        .zip(&values[lag..])
        .map(|(left, right)| left * right)
        .sum()
}

// Integer FFT-hop lags are too coarse for a grid spanning a whole track.
// Repeated pulses at longer lags resolve the period between two FFT hops.
fn refine_period(onset: &[f32], coarse: usize) -> f64 {
    let correlations: Vec<_> = (0..onset.len().min((coarse + 2) * 16 + 1))
        .map(|lag| autocorrelation(onset, lag) / (onset.len() - lag) as f32)
        .collect();
    (-100..=100)
        .map(|offset| coarse as f64 + f64::from(offset) / 100.0)
        .filter(|period| *period >= 1.0)
        .max_by(|&left, &right| {
            period_score(&correlations, left).total_cmp(&period_score(&correlations, right))
        })
        .unwrap_or(coarse as f64)
}

fn period_score(correlations: &[f32], period: f64) -> f32 {
    PERIOD_MULTIPLES
        .into_iter()
        .filter_map(|multiple| {
            let lag = period * multiple as f64;
            let index = lag.floor() as usize;
            correlations
                .get(index)
                .zip(correlations.get(index + 1))
                .map(|(&left, &right)| left + (right - left) * (lag - index as f64) as f32)
        })
        .sum()
}

fn refine_grid(onset: &[f32], approximate_period: f64) -> (f64, usize) {
    // Fit period and phase together over the whole track. Even a 0.1 BPM
    // error can shift a later repeated phrase onto a different beat. Smoothing
    // makes the fit tolerant of the frame quantisation of individual attacks.
    let smoothed: Vec<_> = (0..onset.len())
        .map(|index| {
            0.5 * onset[index]
                + 0.25 * index.checked_sub(1).map_or(0.0, |i| onset[i])
                + 0.25 * onset.get(index + 1).copied().unwrap_or(0.0)
        })
        .collect();
    (-100..=100)
        .map(|offset| approximate_period + f64::from(offset) / 1000.0)
        .filter(|period| *period >= 1.0)
        .map(|period| {
            let phase = estimate_phase(&smoothed, period);
            (period, phase)
        })
        .max_by(|&(left_period, left_phase), &(right_period, right_phase)| {
            phase_score(&smoothed, left_phase, left_period).total_cmp(&phase_score(
                &smoothed,
                right_phase,
                right_period,
            ))
        })
        .unwrap_or((approximate_period, 0))
}

fn estimate_phase(onset: &[f32], period: f64) -> usize {
    (0..(period.ceil() as usize).min(onset.len()))
        .max_by(|&left, &right| {
            phase_score(onset, left, period).total_cmp(&phase_score(onset, right, period))
        })
        .unwrap_or(0)
}

fn phase_score(onset: &[f32], phase: usize, period: f64) -> f32 {
    (0..onset.len())
        .map(|beat| (phase as f64 + beat as f64 * period).round() as usize)
        .take_while(|&index| index < onset.len())
        .map(|index| onset[index])
        .sum()
}

fn track_beat_positions(onset: &[f32], first: usize, period: f64) -> Vec<usize> {
    let period = period.max(1.0);
    let search_radius = (period as usize / 8).max(1);
    let mut positions = Vec::new();
    for beat in 0..onset.len() {
        // Snap each attack independently. Feeding the last snapped onset back
        // into the next prediction accumulates timing errors in breakdowns
        // and can permanently change the beat's position within the bar.
        let predicted = (first as f64 + beat as f64 * period).round() as usize;
        if predicted >= onset.len() {
            break;
        }
        let current = strongest_near(onset, predicted, search_radius);
        if positions.last().is_none_or(|last| current > *last) {
            positions.push(current);
        }
    }
    if positions
        .first()
        .is_some_and(|position| *position as f64 > period / 2.0)
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
    let mut spectral_bands = [0.0; SPECTRAL_BAND_COUNT];
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
        for (total, value) in spectral_bands.iter_mut().zip(frame.spectral_bands) {
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
    for value in &mut spectral_bands {
        *value /= frames.len() as f32;
    }

    let average_rms = frames.iter().map(|frame| frame.rms).sum::<f32>() / frames.len() as f32;
    let onset_strength = frames.iter().map(|frame| frame.flux).sum::<f32>() / frames.len() as f32;
    let mut onset_profile = [0.0; ONSET_PROFILE_SIZE];
    let mut onset_counts = [0_usize; ONSET_PROFILE_SIZE];
    for (index, frame) in frames.iter().enumerate() {
        let section = (index * onset_profile.len() / frames.len()).min(onset_profile.len() - 1);
        onset_profile[section] += frame.flux;
        onset_counts[section] += 1;
    }
    for (value, count) in onset_profile.iter_mut().zip(onset_counts) {
        *value /= count.max(1) as f32;
    }
    Features {
        chroma,
        timbre: [
            centroid / nyquist,
            bandwidth / nyquist,
            rolloff_sum / frames.len() as f32 / nyquist,
            flatness,
        ],
        spectral_bands,
        loudness_db: 20.0 * average_rms.max(1.0e-9).log10(),
        onset_strength,
        onset_profile,
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
        assert_eq!(estimate_phase(&values, 10.0), 3);
    }

    #[test]
    fn tempo_prefers_supported_weak_intermediate_beats() {
        let mut values = vec![0.0; 100];
        for (pulse, index) in (0..100).step_by(10).enumerate() {
            values[index] = if pulse % 2 == 0 { 1.0 } else { 0.8 };
        }

        assert_eq!(prefer_supported_faster_pulse(&values, 20, 5), 10);
    }

    #[test]
    fn frequencies_are_assigned_to_stable_perceptual_bands() {
        assert_eq!(spectral_band(40.0), 0);
        assert_eq!(spectral_band(80.0), 1);
        assert_eq!(spectral_band(199.0), 1);
        assert_eq!(spectral_band(200.0), 2);
        assert_eq!(spectral_band(12_000.0), SPECTRAL_BAND_COUNT - 1);
    }

    #[test]
    fn beat_tracker_follows_local_timing_changes() {
        let mut onset = vec![0.0; 70];
        for position in [3, 14, 24, 35, 45, 56, 66] {
            onset[position] = 1.0;
        }
        assert_eq!(
            track_beat_positions(&onset, 3, 10.5),
            vec![3, 14, 24, 35, 45, 56, 66]
        );
    }

    #[test]
    fn fractional_tempo_fit_preserves_distant_repeated_beats() {
        let period = 29.531;
        let phase: usize = 3;
        let beat_count = 600;
        let mut onset = vec![0.0; (period * beat_count as f64).ceil() as usize];
        for beat in 0..beat_count {
            if (140..220).contains(&beat) {
                continue; // A breakdown must not change the subsequent bar phase.
            }
            let position = (phase as f64 + beat as f64 * period).round() as usize;
            if let Some(value) = onset.get_mut(position) {
                *value = if beat % 2 == 0 { 1.0 } else { 0.8 };
            }
        }

        let (fitted_period, first) = refine_grid(&onset, refine_period(&onset, 30));
        assert!((fitted_period - period).abs() < 0.005);
        let positions = track_beat_positions(&onset, first, fitted_period);
        assert_eq!(positions.len(), beat_count);
        for beat in [20, 276, 532] {
            let expected = (phase as f64 + beat as f64 * period).round() as usize;
            assert!(positions[beat].abs_diff(expected) <= 1);
        }
    }

    #[test]
    fn transient_offsets_do_not_accumulate_across_a_breakdown() {
        let mut onset = vec![0.0; 6_000];
        for beat in 0..200 {
            let position = 3 + beat * 30;
            if beat < 50 {
                onset[position + 2] = 1.0;
            } else if beat >= 150 {
                onset[position] = 1.0;
            }
        }
        let positions = track_beat_positions(&onset, 3, 30.0);
        assert_eq!(positions.len(), 200);
        assert_eq!(positions[175], 3 + 175 * 30);
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
