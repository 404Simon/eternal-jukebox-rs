import { describe, expect, test } from 'vitest'
import { beatLabel, formatSession, formatTime } from './format'

describe('formatTime', () => {
  test('renders minutes and zero-padded seconds', () => {
    expect(formatTime(0)).toBe('0:00')
    expect(formatTime(9)).toBe('0:09')
    expect(formatTime(65)).toBe('1:05')
    expect(formatTime(600)).toBe('10:00')
  })
})

describe('formatSession', () => {
  test('renders a zero-padded hh:mm:ss clock', () => {
    expect(formatSession(0)).toBe('00:00:00')
    expect(formatSession(59)).toBe('00:00:59')
    expect(formatSession(3661)).toBe('01:01:01')
  })
})

describe('beatLabel', () => {
  test('pads beat indices to three digits', () => {
    expect(beatLabel(0)).toBe('000')
    expect(beatLabel(7)).toBe('007')
    expect(beatLabel(1234)).toBe('1234')
  })
})
