import type { Beat, PlannedBeat } from './types'

const LOOK_AHEAD_SECONDS = 3
const SCHEDULER_INTERVAL_MS = 100
const FADE_SECONDS = 0.002

export class EndlessPlayer {
  private readonly gain: GainNode
  private sources = new Set<AudioBufferSourceNode>()
  private timer: number | undefined
  private cursor = 0
  private generation = 0
  private pending: PlannedBeat | undefined
  private scheduled: PlannedBeat[] = []

  constructor(
    readonly context: AudioContext,
    private readonly buffer: AudioBuffer,
    private readonly beats: Beat[],
    private readonly nextBeat: () => PlannedBeat,
    private readonly onBeat: (step: PlannedBeat) => void,
    private readonly onQueue: (steps: PlannedBeat[]) => void,
    initialVolume: number,
  ) {
    this.gain = context.createGain()
    this.gain.connect(this.context.destination)
    this.gain.gain.value = initialVolume
  }

  get paused() { return this.context.state !== 'running' }

  async start() {
    await this.context.resume()
    this.cursor = this.context.currentTime + 0.08
    this.fillQueue()
    this.timer = window.setInterval(() => this.fillQueue(), SCHEDULER_INTERVAL_MS)
  }

  async toggle() {
    if (this.paused) await this.context.resume()
    else await this.context.suspend()
  }

  setVolume(value: number) {
    this.gain.gain.setTargetAtTime(value, this.context.currentTime, 0.01)
  }

  seek() {
    this.generation++
    this.sources.forEach((source) => source.stop())
    this.sources.clear()
    this.pending = undefined
    this.scheduled = []
    this.onQueue([])
    this.cursor = this.context.currentTime + 0.05
    this.fillQueue()
  }

  destroy() {
    if (this.timer !== undefined) clearInterval(this.timer)
    this.generation++
    this.sources.forEach((source) => source.stop())
    this.sources.clear()
    this.scheduled = []
    void this.context.close()
  }

  private fillQueue() {
    this.pending ??= this.nextBeat()
    while (this.cursor < this.context.currentTime + LOOK_AHEAD_SECONDS) {
      const step = this.pending
      const next = this.nextBeat()
      const beat = this.beats[step.beat]
      if (!beat) return
      this.schedule(step, beat, next)
      this.pending = next
    }
  }

  private schedule(step: PlannedBeat, beat: Beat, next: PlannedBeat) {
    const source = this.context.createBufferSource()
    const envelope = this.context.createGain()
    const startFrame = Math.round(beat.start * this.buffer.sampleRate)
    const requestedEndFrame = Math.round((beat.start + beat.duration) * this.buffer.sampleRate)
    const endFrame = Math.min(requestedEndFrame, this.buffer.length)
    const offset = startFrame / this.buffer.sampleRate
    const duration = Math.max(0, endFrame - startFrame) / this.buffer.sampleRate
    const fade = Math.min(FADE_SECONDS, duration / 3)
    const generation = this.generation
    source.buffer = this.buffer
    source.connect(envelope).connect(this.gain)
    envelope.gain.setValueAtTime(step.jumpedFrom === null ? 1 : 0, this.cursor)
    if (step.jumpedFrom !== null) {
      envelope.gain.linearRampToValueAtTime(1, this.cursor + fade)
    }
    if (next.jumpedFrom !== null) {
      envelope.gain.setValueAtTime(1, this.cursor + duration - fade)
      envelope.gain.linearRampToValueAtTime(0, this.cursor + duration)
    }
    source.start(this.cursor, offset, duration)
    source.stop(this.cursor + duration)
    const delay = Math.max(0, (this.cursor - this.context.currentTime) * 1000)
    this.scheduled.push(step)
    this.onQueue([...this.scheduled])
    window.setTimeout(() => {
      if (generation !== this.generation) return
      this.scheduled.shift()
      this.onQueue([...this.scheduled])
      this.onBeat(step)
    }, delay)
    this.sources.add(source)
    source.onended = () => this.sources.delete(source)
    this.cursor += duration
  }
}

export function interleave(buffer: AudioBuffer): Float32Array {
  const output = new Float32Array(buffer.length * buffer.numberOfChannels)
  for (let channel = 0; channel < buffer.numberOfChannels; channel++) {
    const input = buffer.getChannelData(channel)
    for (let frame = 0; frame < buffer.length; frame++) {
      output[frame * buffer.numberOfChannels + channel] = input[frame] ?? 0
    }
  }
  return output
}
