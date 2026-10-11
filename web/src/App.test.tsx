import { render } from 'vitest-browser-react'
import { userEvent } from 'vitest/browser'
import { afterEach, beforeEach, expect, test, vi } from 'vitest'
import App from './App'
import { FakeAudioBuffer, makePreparedTrack } from './test/fake-audio'
import type { PreparedTrack } from './types'

const state = vi.hoisted(() => ({
  prepared: undefined as PreparedTrack | undefined,
  analyse: undefined as (() => Promise<PreparedTrack>) | undefined,
}))

vi.mock('./analysis', () => ({
  analyseBuffer: () => (state.analyse ? state.analyse() : Promise.resolve(state.prepared as PreparedTrack)),
}))

vi.mock('./wasm/eternal_web', () => ({
  default: async () => {},
  WebPlanner: class {
    plan_next() { return '[0,null,1]' }
    probabilities() { return '[]' }
    continue_from() {}
    free() {}
  },
}))

vi.mock('./audio', () => ({
  EndlessPlayer: class {
    paused = false
    async start() {}
    async toggle() { this.paused = !this.paused }
    setVolume() {}
    seek() {}
    destroy() {}
  },
}))

class StubAudioContext {
  state: AudioContextState = 'running'
  async decodeAudioData() { return new FakeAudioBuffer().asAudioBuffer() }
  async close() { this.state = 'closed' }
}

const sessionText = (container: HTMLElement) =>
  container.querySelector('.session-time strong')?.textContent ?? ''

beforeEach(() => {
  state.prepared = makePreparedTrack(16)
  state.analyse = undefined
  vi.stubGlobal('AudioContext', StubAudioContext)
})

afterEach(() => {
  vi.unstubAllGlobals()
})

test('the session clock stops advancing while playback is paused', async () => {
  const screen = await render(<App />)

  const input = screen.container.querySelector('input[type=file]')
  if (!(input instanceof HTMLInputElement)) throw new Error('file input not found')
  await userEvent.upload(input, new File([new Uint8Array([1, 2, 3])], 'demo.mp3', { type: 'audio/mpeg' }))

  await expect.element(screen.getByRole('button', { name: 'Pause' })).toBeVisible()

  await vi.waitFor(() => {
    expect(sessionText(screen.container)).not.toBe('00:00:00')
  }, { timeout: 3000, interval: 100 })

  await screen.getByRole('button', { name: 'Pause' }).click()
  await expect.element(screen.getByRole('button', { name: 'Play' })).toBeVisible()

  const frozen = sessionText(screen.container)
  await new Promise((resolve) => setTimeout(resolve, 1500))

  expect(sessionText(screen.container)).toBe(frozen)
})

test('keyboard shortcuts work while a range slider has focus', async () => {
  const screen = await render(<App />)

  const input = screen.container.querySelector('input[type=file]')
  if (!(input instanceof HTMLInputElement)) throw new Error('file input not found')
  await userEvent.upload(input, new File([new Uint8Array([1])], 'demo.mp3', { type: 'audio/mpeg' }))
  await expect.element(screen.getByRole('button', { name: 'Pause' })).toBeVisible()

  const volume = screen.container.querySelector('.volume-control input')
  if (!(volume instanceof HTMLInputElement)) throw new Error('volume slider not found')
  volume.focus()
  await userEvent.keyboard('p')

  await expect.element(screen.getByRole('button', { name: 'Play' })).toBeVisible()
})

test('a superseded load never clobbers the newest track', async () => {
  const resolvers: Array<(value: PreparedTrack) => void> = []
  state.analyse = () => new Promise((resolve) => { resolvers.push(resolve) })

  const screen = await render(<App />)
  const input = screen.container.querySelector('input[type=file]')
  if (!(input instanceof HTMLInputElement)) throw new Error('file input not found')

  await userEvent.upload(input, new File([new Uint8Array([1])], 'first.mp3', { type: 'audio/mpeg' }))
  await vi.waitFor(() => { expect(resolvers).toHaveLength(1) })
  await userEvent.upload(input, new File([new Uint8Array([2])], 'second.mp3', { type: 'audio/mpeg' }))
  await vi.waitFor(() => { expect(resolvers).toHaveLength(2) })

  const first = makePreparedTrack(16)
  first.analysis.tempo = 111
  const second = makePreparedTrack(16)
  second.analysis.tempo = 222

  resolvers[1]?.(second)
  resolvers[0]?.(first)

  await expect.element(screen.getByText('second.mp3')).toBeVisible()
  await expect.element(screen.getByText(/222\.0 BPM/)).toBeVisible()
})
