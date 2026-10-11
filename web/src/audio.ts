import type { Beat, PlannedBeat } from './types'

const LOOK_AHEAD_SECONDS = 3
const SCHEDULER_INTERVAL_MS = 100
const FADE_SECONDS = 0.002

interface ScheduledBeat {
  step: PlannedBeat
  at: number
}

export class EndlessPlayer {
  private readonly gain: GainNode
  private sources = new Set<AudioBufferSourceNode>()
  private timer: number | undefined
  private cursor = 0
  private pending: PlannedBeat | undefined
  private timeline: ScheduledBeat[] = []

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
    this.timer = window.setInterval(() => this.tick(), SCHEDULER_INTERVAL_MS)
  }

  async toggle() {
    if (this.paused) await this.context.resume()
    else await this.context.suspend()
  }

  setVolume(value: number) {
    this.gain.gain.setTargetAtTime(value, this.context.currentTime, 0.01)
  }

  seek() {
    this.sources.forEach((source) => source.stop())
    this.sources.clear()
    this.pending = undefined
    this.timeline = []
    this.onQueue([])
    this.cursor = this.context.currentTime + 0.05
    this.fillQueue()
  }

  destroy() {
    if (this.timer !== undefined) clearInterval(this.timer)
    this.sources.forEach((source) => source.stop())
    this.sources.clear()
    this.timeline = []
    void this.context.close()
  }

  private tick() {
    this.fillQueue()
    for (let head = this.timeline[0]; head && head.at <= this.context.currentTime; head = this.timeline[0]) {
      this.timeline.shift()
      this.onBeat(head.step)
      this.onQueue(this.timeline.map((entry) => entry.step))
    }
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
    this.timeline.push({ step, at: this.cursor })
    this.onQueue(this.timeline.map((entry) => entry.step))
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
