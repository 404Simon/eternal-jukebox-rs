/// <reference lib="webworker" />
import init, { prepare_track } from './wasm/eternal_web'

self.onmessage = async (event: MessageEvent<{ samples: Float32Array; sampleRate: number; channels: number }>) => {
  try {
    await init()
    const { samples, sampleRate, channels } = event.data
    const prepared = prepare_track(samples, sampleRate, channels)
    self.postMessage({ prepared })
  } catch (error) {
    self.postMessage({ error: error instanceof Error ? error.message : String(error) })
  }
}
