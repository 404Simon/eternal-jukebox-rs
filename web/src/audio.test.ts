import { afterEach, describe, expect, test, vi } from 'vitest'
import { EndlessPlayer } from './audio'
import { FakeAudioBuffer, FakeAudioContext, makeBeats } from './test/fake-audio'
import type { PlannedBeat } from './types'

function setup() {
  const context = new FakeAudioContext(new FakeAudioBuffer())
  const beats = makeBeats(64)
  const events: PlannedBeat[] = []
  let next = 0
  const player = new EndlessPlayer(
    context.asAudioContext(),
    context.buffer.asAudioBuffer(),
    beats,
    () => ({ beat: next++, jumpedFrom: null, probability: 1 }),
    (step) => events.push(step),
    () => {},
    1,
  )
  return { context, player, events }
}

afterEach(() => {
  vi.useRealTimers()
})

describe('EndlessPlayer pause semantics', () => {
  test('does not report played beats while the context is suspended', async () => {
    vi.useFakeTimers()
    const { player, events } = setup()

    await player.start()
    await player.toggle()
    expect(player.paused).toBe(true)

    await vi.advanceTimersByTimeAsync(5000)

    expect(events).toHaveLength(0)
  })

  test('reports played beats once the audio clock advances', async () => {
    vi.useFakeTimers()
    const { context, player, events } = setup()

    await player.start()
    context.currentTime = 1.5
    await vi.advanceTimersByTimeAsync(150)

    expect(events.length).toBeGreaterThanOrEqual(1)
  })
})
