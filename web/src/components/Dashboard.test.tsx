import type { ComponentProps } from 'react'
import { render } from 'vitest-browser-react'
import { expect, test, vi } from 'vitest'
import { Dashboard } from './Dashboard'
import { makePreparedTrack } from '../test/fake-audio'

type Props = ComponentProps<typeof Dashboard>

const base: Props = {
  choices: [{ source: 0, destination: 2, probability: 0.6, distance: 2, is_sequential: false }],
  coverage: Array<number>(8).fill(0),
  fileName: 'demo.mp3',
  history: [],
  jumpCount: 0,
  live: { beat: 0, jumpedFrom: null, probability: 1 },
  paused: false,
  queue: [],
  sessionSeconds: 0,
  track: makePreparedTrack(8),
  volume: 0.85,
  onChangeTrack: vi.fn(),
  onSeek: vi.fn(),
  onSeekToBeat: vi.fn(),
  onToggle: vi.fn(),
  onVolume: vi.fn(),
}

const makeProps = (overrides: Partial<Props> = {}): Props => ({ ...base, ...overrides })

test('renders the track, session clock and playing transport', async () => {
  const screen = await render(<Dashboard {...makeProps({ sessionSeconds: 3661 })} />)

  await expect.element(screen.getByText('demo.mp3')).toBeVisible()
  await expect.element(screen.getByText('01:01:01')).toBeVisible()
  await expect.element(screen.getByRole('button', { name: 'Pause' })).toBeVisible()
})

test('reflects the paused state and reports toggle clicks', async () => {
  const onToggle = vi.fn()
  const screen = await render(<Dashboard {...makeProps({ paused: true, onToggle })} />)

  await expect.element(screen.getByText('Paused')).toBeVisible()
  await screen.getByRole('button', { name: 'Play' }).click()

  expect(onToggle).toHaveBeenCalledTimes(1)
})

test('seek buttons report -10 and +10 second offsets', async () => {
  const onSeek = vi.fn()
  const screen = await render(<Dashboard {...makeProps({ onSeek })} />)

  await screen.getByRole('button', { name: 'Back 10 seconds' }).click()
  await screen.getByRole('button', { name: 'Forward 10 seconds' }).click()

  expect(onSeek).toHaveBeenNthCalledWith(1, -10)
  expect(onSeek).toHaveBeenNthCalledWith(2, 10)
})
