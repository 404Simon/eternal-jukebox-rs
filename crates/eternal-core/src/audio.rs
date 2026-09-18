use std::{fs::File, path::Path};

use symphonia::core::{
    audio::SampleBuffer,
    codecs::DecoderOptions,
    errors::Error as SymphoniaError,
    formats::FormatOptions,
    io::{MediaSourceStream, MediaSourceStreamOptions},
    meta::MetadataOptions,
    probe::Hint,
};
use thiserror::Error;

/// Fully decoded, interleaved PCM audio.
#[derive(Clone, Debug)]
pub struct Audio {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
    pub channels: u16,
}

impl Audio {
    #[must_use]
    pub fn duration_seconds(&self) -> f64 {
        self.samples.len() as f64 / f64::from(self.channels) / f64::from(self.sample_rate)
    }

    /// Mix all channels down to mono without changing the sample rate.
    #[must_use]
    pub fn mono(&self) -> Vec<f32> {
        let channels = usize::from(self.channels);
        self.samples
            .chunks_exact(channels)
            .map(|frame| frame.iter().sum::<f32>() / channels as f32)
            .collect()
    }
}

#[derive(Debug, Error)]
pub enum AudioError {
    #[error("could not open {path}: {source}")]
    Open {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("unsupported or unrecognised audio file: {0}")]
    Probe(#[source] SymphoniaError),
    #[error("the audio file contains no decodable track")]
    NoTrack,
    #[error("audio decoder error: {0}")]
    Decode(#[source] SymphoniaError),
    #[error("audio stream has no channel or sample-rate information")]
    MissingSignalSpec,
    #[error("audio stream changes format partway through the file")]
    FormatChanged,
}

/// Decode an MP3 (or another format enabled by Symphonia) into interleaved PCM.
pub fn decode(path: impl AsRef<Path>) -> Result<Audio, AudioError> {
    let path = path.as_ref();
    let file = File::open(path).map_err(|source| AudioError::Open {
        path: path.display().to_string(),
        source,
    })?;
    let mut hint = Hint::new();
    if let Some(extension) = path.extension().and_then(|value| value.to_str()) {
        hint.with_extension(extension);
    }

    let source = MediaSourceStream::new(Box::new(file), MediaSourceStreamOptions::default());
    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            source,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(AudioError::Probe)?;
    let mut format = probed.format;
    let track = format.default_track().ok_or(AudioError::NoTrack)?;
    let track_id = track.id;
    let mut codec = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(AudioError::Decode)?;

    let mut output = Vec::new();
    let mut signal_spec = None;
    while let Ok(packet) = format.next_packet() {
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = match codec.decode(&packet) {
            Ok(decoded) => decoded,
            Err(SymphoniaError::DecodeError(_)) => continue,
            Err(source) => return Err(AudioError::Decode(source)),
        };
        let spec = *decoded.spec();
        if signal_spec.is_some_and(|previous| previous != spec) {
            return Err(AudioError::FormatChanged);
        }
        signal_spec = Some(spec);
        let mut samples = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
        samples.copy_interleaved_ref(decoded);
        output.extend_from_slice(samples.samples());
    }

    let spec = signal_spec.ok_or(AudioError::MissingSignalSpec)?;
    Ok(Audio {
        samples: output,
        sample_rate: spec.rate,
        channels: spec.channels.count() as u16,
    })
}
