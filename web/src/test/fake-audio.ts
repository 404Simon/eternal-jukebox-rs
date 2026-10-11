import type { Analysis, Beat, BranchGraph, PreparedTrack } from '../types'

class FakeAudioParam {
  value = 1
  setValueAtTime(): void {}
  linearRampToValueAtTime(): void {}
  setTargetAtTime(): void {}
}

class FakeGainNode {
  readonly gain = new FakeAudioParam()
  connect<T>(node: T): T {
    return node
  }
}

class FakeBufferSource {
  buffer: unknown = null
  onended: (() => void) | null = null
  startedAt: number | null = null
  stoppedAt: number | null = null
  connect<T>(node: T): T {
    return node
  }
  start(when = 0): void {
    this.startedAt = when
  }
  stop(when = 0): void {
    this.stoppedAt = when
  }
}

export class FakeAudioContext {
  state: AudioContextState = 'running'
  currentTime = 0
  readonly sampleRate = 8000
  readonly destination = {}
  readonly gains: FakeGainNode[] = []
  readonly sources: FakeBufferSource[] = []
  readonly buffer: FakeAudioBuffer

  constructor(buffer = new FakeAudioBuffer()) {
    this.buffer = buffer
  }

  createGain(): FakeGainNode {
    const gain = new FakeGainNode()
    this.gains.push(gain)
    return gain
  }

  createBufferSource(): FakeBufferSource {
    const source = new FakeBufferSource()
    this.sources.push(source)
    return source
  }

  async resume(): Promise<void> {
    this.state = 'running'
  }

  async suspend(): Promise<void> {
    this.state = 'suspended'
  }

  async close(): Promise<void> {
    this.state = 'closed'
  }

  async decodeAudioData(): Promise<FakeAudioBuffer> {
    return this.buffer
  }

  asAudioContext(): AudioContext {
    return this as unknown as AudioContext
  }
}

export class FakeAudioBuffer {
  readonly sampleRate: number
  readonly length: number
  readonly numberOfChannels = 1

  constructor(sampleRate = 8000, seconds = 120) {
    this.sampleRate = sampleRate
    this.length = sampleRate * seconds
  }

  getChannelData(): Float32Array {
    return new Float32Array(this.length)
  }

  asAudioBuffer(): AudioBuffer {
    return this as unknown as AudioBuffer
  }
}

export function makeBeats(count: number, duration = 1): Beat[] {
  return Array.from({ length: count }, (_, index) => ({ index, start: index * duration, duration }))
}

export function makePreparedTrack(beatCount = 16): PreparedTrack {
  const analysis: Analysis = {
    duration: beatCount,
    sample_rate: 8000,
    channels: 1,
    tempo: 120,
    beats: makeBeats(beatCount),
  }
  const branches: BranchGraph['branches'] = analysis.beats.map((beat, index) => (
    index < analysis.beats.length - 2 ? [{ destination: index + 2, distance: 2 }] : []
  ))
  return { analysis, graph: { branches, threshold: 4, last_branch_point: 0 } }
}
