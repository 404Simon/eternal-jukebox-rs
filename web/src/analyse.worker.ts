/// <reference lib="webworker" />
import init, { prepare_track } from './wasm/eternal_web'

self.onmessage = async (event: MessageEvent<{ samples: Float32Array; sampleRate: number; channels: number; threshold?: number | null; islandThresholdSeconds?: number | null }>) => {
  try {
    await init()
    const { samples, sampleRate, channels, threshold, islandThresholdSeconds } = event.data
    const prepared = prepare_track(samples, sampleRate, channels, threshold ?? undefined, islandThresholdSeconds ?? undefined)
    self.postMessage({ prepared })
  } catch (error) {
    self.postMessage({ error: error instanceof Error ? error.message : String(error) })
  }
}
