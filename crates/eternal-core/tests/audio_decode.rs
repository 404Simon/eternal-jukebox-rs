#![cfg(not(target_arch = "wasm32"))]

use eternal_core::decode;

#[test]
fn decodes_mp3_and_opus_into_interleaved_stereo() {
    for filename in ["stereo.mp3", "stereo.opus"] {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(filename);
        let audio = decode(path).expect("fixture should decode");
        assert_eq!(audio.sample_rate, 48_000, "{filename}");
        assert_eq!(audio.channels, 2, "{filename}");
        assert_eq!(audio.samples.len() % 2, 0, "{filename}");
        assert!(
            (0.19..0.26).contains(&audio.duration_seconds()),
            "{filename}: {} seconds",
            audio.duration_seconds()
        );
        assert!(audio.samples.iter().all(|sample| sample.is_finite()));

        // The left channel is 440 Hz; the right is 880 Hz. A planar copy or
        // swapped channels would change these counts in the middle of the file.
        let frames = audio
            .samples
            .as_chunks::<2>()
            .0
            .iter()
            .skip(2_400)
            .take(4_800);
        let (mut left_crossings, mut right_crossings) = (0, 0);
        let mut previous = [0.0, 0.0];
        for frame in frames {
            left_crossings += usize::from(previous[0] < 0.0 && frame[0] >= 0.0);
            right_crossings += usize::from(previous[1] < 0.0 && frame[1] >= 0.0);
            previous.copy_from_slice(frame);
        }
        assert!((43..=45).contains(&left_crossings), "{filename}");
        assert!((87..=89).contains(&right_crossings), "{filename}");
    }
}
