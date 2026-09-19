import { interleave } from './audio'
import type { PreparedTrack } from './types'

interface WorkerResponse {
  error?: string
  prepared?: string
}

export async function analyseBuffer(buffer: AudioBuffer): Promise<PreparedTrack> {
  const samples = interleave(buffer)
  const prepared = await new Promise<string>((resolve, reject) => {
    const worker = new Worker(new URL('./analyse.worker.ts', import.meta.url), { type: 'module' })
    const stop = () => worker.terminate()

    worker.onmessage = ({ data }: MessageEvent<WorkerResponse>) => {
      stop()
      if (data.error) reject(new Error(data.error))
      else if (data.prepared) resolve(data.prepared)
      else reject(new Error('The analyser returned no result.'))
    }
    worker.onerror = (event) => {
      stop()
      reject(new Error(event.message))
    }
    worker.postMessage(
      { samples, sampleRate: buffer.sampleRate, channels: buffer.numberOfChannels },
      [samples.buffer],
    )
  })
  return JSON.parse(prepared) as PreparedTrack
}
