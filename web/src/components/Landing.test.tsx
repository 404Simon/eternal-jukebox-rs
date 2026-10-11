import type { ComponentProps } from 'react'
import { render } from 'vitest-browser-react'
import { expect, test, vi } from 'vitest'
import { Landing } from './Landing'

type Props = ComponentProps<typeof Landing>

const makeProps = (overrides: Partial<Props> = {}): Props => ({
  dragging: false,
  error: '',
  fileName: '',
  phase: 'idle',
  onDragChange: vi.fn(),
  onFile: vi.fn(),
  ...overrides,
})

test('keeps the drop highlight while dragging over child elements', async () => {
  const onDragChange = vi.fn()
  const screen = await render(<Landing {...makeProps({ onDragChange })} />)

  const label = screen.container.querySelector('.dropzone')
  const child = screen.container.querySelector('.dropzone strong')
  if (!(label instanceof HTMLElement) || !(child instanceof HTMLElement)) throw new Error('dropzone not found')

  label.dispatchEvent(new DragEvent('dragenter', { bubbles: true }))
  child.dispatchEvent(new DragEvent('dragenter', { bubbles: true }))
  child.dispatchEvent(new DragEvent('dragleave', { bubbles: true }))

  expect(onDragChange).toHaveBeenLastCalledWith(true)

  label.dispatchEvent(new DragEvent('dragleave', { bubbles: true }))
  expect(onDragChange).toHaveBeenLastCalledWith(false)
})
