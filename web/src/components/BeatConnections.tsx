import { useId, useMemo } from 'react'
import { beatLabel } from '../format'
import type { BranchGraph, PlannedBeat } from '../types'

const WIDTH = 1000
const HEIGHT = 180

function position(beat: number, count: number) {
  return Math.max(0, Math.min(beat, count - 1)) / Math.max(1, count - 1) * WIDTH
}

function connection(source: number, destination: number, count: number) {
  // Hide the automatic wrap in every layer without changing playback.
  if (source === count - 1 && destination === 0) return ''
  const from = position(source, count)
  const to = position(destination, count)
  const top = HEIGHT - Math.sqrt(Math.abs(to - from) / WIDTH) * HEIGHT * 1.18
  return `M${from},${HEIGHT} C${from},${top} ${to},${top} ${to},${HEIGHT}`
}

export function BeatConnections({ graph, count, currentBeat, queue }: {
  graph: BranchGraph
  count: number
  currentBeat: number
  queue: PlannedBeat[]
}) {
  const id = useId()
  const background = useMemo(() => {
    // Merge connections at display resolution and cache the static graph.
    const seen = new Set<string>()
    const paths: string[] = []
    let maxSpan = 0
    graph.branches.forEach((branches, source) => {
      for (const { destination } of branches) {
        if (source === count - 1 && destination === 0) continue
        maxSpan = Math.max(maxSpan, Math.abs(position(source, count) - position(destination, count)))
        const from = Math.round(position(source, count) / 6)
        const to = Math.round(position(destination, count) / 6)
        const key = `${Math.min(from, to)}-${Math.max(from, to)}`
        if (!seen.has(key)) {
          seen.add(key)
          paths.push(connection(source, destination, count))
        }
      }
    })
    return { path: paths.join(' '), maxSpan }
  }, [graph, count])
  // The web scheduler removes the live beat from the queue when it starts.
  const nextJump = queue.find((step) => step.jumpedFrom !== null)
  const branches = graph.branches[currentBeat] ?? []
  const cursor = position(currentBeat, count)
  const nextSource = nextJump?.jumpedFrom
  const nextSpan = nextJump && nextSource != null && !(nextSource === count - 1 && nextJump.beat === 0)
    ? Math.abs(position(nextSource, count) - position(nextJump.beat, count)) : 0
  // A cubic's peak is 3/4 of its control height. Fit it with 4% headroom.
  const visibleHeight = Math.max(1, Math.sqrt(Math.max(background.maxSpan, nextSpan) / WIDTH) * HEIGHT * 1.18 * .75 / .96)
  const viewTop = HEIGHT - visibleHeight
  const nextLabel = nextJump && nextSource != null
    ? `Next jump: ${beatLabel(nextSource)} → ${beatLabel(nextJump.beat)}`
    : 'No jump queued'

  return <div className="beat-connections">
    <svg viewBox={`0 ${viewTop} ${WIDTH} ${visibleHeight}`} preserveAspectRatio="none" role="img"
      aria-label={`Beat connections. Current beat ${beatLabel(currentBeat)}, ${branches.length} available paths. ${nextLabel}`}>
      <defs>
        <linearGradient id={`${id}-paths`} x1="0" y1="0" x2="0" y2="1">
          <stop offset="0" stopColor="#8994bd" stopOpacity=".38" />
          <stop offset="1" stopColor="#8994bd" stopOpacity=".06" />
        </linearGradient>
        <linearGradient id={`${id}-cursor`} x1="0" y1="0" x2="0" y2="1">
          <stop offset="0" stopColor="#89dceb" stopOpacity="0" />
          <stop offset="1" stopColor="#89dceb" stopOpacity=".65" />
        </linearGradient>
      </defs>
      <g fill="none" strokeWidth="1" strokeLinecap="round">
        <path d={background.path} stroke={`url(#${id}-paths)`} vectorEffect="non-scaling-stroke" />
        {count > 0 && <path stroke={`url(#${id}-cursor)`} d={`M${cursor},${HEIGHT} V${viewTop + visibleHeight * .04}`} vectorEffect="non-scaling-stroke" />}
        <path className="connection-live" d={branches.map(({ destination }) => connection(currentBeat, destination, count)).join(' ')} vectorEffect="non-scaling-stroke" />
        {nextJump && nextSource != null && <>
          <path className="connection-next-halo" d={connection(nextSource, nextJump.beat, count)} strokeWidth="6" vectorEffect="non-scaling-stroke" />
          <path className="connection-next" d={connection(nextSource, nextJump.beat, count)} strokeWidth="1.75" vectorEffect="non-scaling-stroke" />
        </>}
      </g>
    </svg>
  </div>
}
