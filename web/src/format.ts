export function formatTime(seconds: number) {
  const minutes = Math.floor(seconds / 60)
  return `${minutes}:${Math.floor(seconds % 60).toString().padStart(2, '0')}`
}

export function formatSession(seconds: number) {
  return [Math.floor(seconds / 3600), Math.floor(seconds / 60) % 60, seconds % 60]
    .map((part) => part.toString().padStart(2, '0'))
    .join(':')
}

export function beatLabel(beat: number) {
  return beat.toString().padStart(3, '0')
}
