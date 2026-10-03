These fixtures contain 0.2 seconds of generated stereo tones at 48 kHz:
440 Hz on the left and 880 Hz on the right. No third-party audio is used.

Generate each fixture with FFmpeg (replace `CODEC` and `OUTPUT`):

```sh
ffmpeg -f lavfi -i 'aevalsrc=0.25*sin(2*PI*440*t)|0.25*sin(2*PI*880*t):s=48000:d=0.2' -c:a CODEC OUTPUT
```

- `libmp3lame` → `stereo.mp3`
- `libopus` → `stereo.opus`
