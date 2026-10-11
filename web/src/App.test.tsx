import { render } from 'vitest-browser-react'
import { userEvent } from 'vitest/browser'
import { afterEach, beforeEach, expect, test, vi } from 'vitest'
import App from './App'
import { FakeAudioBuffer, makePreparedTrack } from './test/fake-audio'

const state = vi.hoisted(() => ({ prepared: undefined as unknown }))

vi.mock('./analysis', () => ({
  analyseBuffer: async () => state.prepared,
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
