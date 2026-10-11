import { useCallback, useEffect, useRef, useState } from 'react'
import initWasm, { WebPlanner } from './wasm/eternal_web'
import { analyseBuffer } from './analysis'
import { EndlessPlayer } from './audio'
import { Dashboard } from './components/Dashboard'
import { Landing } from './components/Landing'
import type { PlannedBeat, PreparedTrack, TransitionProbability } from './types'

type Phase = 'idle' | 'decoding' | 'analysing' | 'ready' | 'error'

function App() {
  const [phase, setPhase] = useState<Phase>('idle')
  const [error, setError] = useState('')
  const [fileName, setFileName] = useState('')
  const [track, setTrack] = useState<PreparedTrack>()
  const [live, setLive] = useState<PlannedBeat>()
  const [history, setHistory] = useState<PlannedBeat[]>([])
  const [jumpCount, setJumpCount] = useState(0)
  const [queue, setQueue] = useState<PlannedBeat[]>([])
  const [coverage, setCoverage] = useState<number[]>([])
  const [choices, setChoices] = useState<TransitionProbability[]>([])
  const [paused, setPaused] = useState(false)
  const [volume, setVolume] = useState(0.85)
  const [dragging, setDragging] = useState(false)
  const [sessionSeconds, setSessionSeconds] = useState(0)
  const sessionElapsed = useRef(0)
  const sessionTick = useRef(0)
  const loadGeneration = useRef(0)
  const player = useRef<EndlessPlayer | undefined>(undefined)
  const planner = useRef<WebPlanner | undefined>(undefined)

  useEffect(() => () => {
    loadGeneration.current++
    player.current?.destroy()
    planner.current?.free()
  }, [])
  useEffect(() => {
    if (!track || paused) return
    sessionTick.current = Date.now()
    const timer = window.setInterval(() => {
      const now = Date.now()
      sessionElapsed.current += (now - sessionTick.current) / 1000
      sessionTick.current = now
      setSessionSeconds(Math.floor(sessionElapsed.current))
    }, 1000)
    return () => clearInterval(timer)
  }, [track, paused])

  const load = useCallback(async (file: File) => {
    const generation = ++loadGeneration.current
    player.current?.destroy()
    planner.current?.free()
    player.current = undefined
    planner.current = undefined
    setError('')
    setTrack(undefined)
    setLive(undefined)
    setHistory([])
    setJumpCount(0)
    setQueue([])
    setCoverage([])
    setChoices([])
    sessionElapsed.current = 0
    sessionTick.current = 0
    setSessionSeconds(0)
    setPaused(false)
    setFileName(file.name)
    let context: AudioContext | undefined
    try {
      setPhase('decoding')
      context = new AudioContext({ latencyHint: 'playback' })
      const buffer = await context.decodeAudioData(await file.arrayBuffer())
      if (generation !== loadGeneration.current) { await context.close(); return }
      setPhase('analysing')
      const result = await analyseBuffer(buffer)
      if (generation !== loadGeneration.current) { await context.close(); return }
      if (!result.graph.branches.some((branches) => branches.length)) {
        throw new Error('No usable transitions found. Try a longer or more repetitive track.')
      }
      await initWasm()
      if (generation !== loadGeneration.current) { await context.close(); return }
      const engine = new WebPlanner(JSON.stringify(result.graph))
      if (generation !== loadGeneration.current) { engine.free(); await context.close(); return }
      planner.current = engine
      const nextBeat = (): PlannedBeat => {
        const [beat, jumpedFrom, probability] = JSON.parse(engine.plan_next()) as [number, number | null, number]
        setChoices(JSON.parse(engine.probabilities()) as TransitionProbability[])
        return { beat, jumpedFrom, probability }
      }
      const audioPlayer = new EndlessPlayer(
        context,
        buffer,
        result.analysis.beats,
        nextBeat,
        (step) => {
          setLive(step)
          setHistory((items) => [step, ...items].slice(0, 256))
          if (step.jumpedFrom !== null) setJumpCount((count) => count + 1)
          setCoverage((items) => {
            const updated = items.length === result.analysis.beats.length
              ? [...items]
              : Array<number>(result.analysis.beats.length).fill(0)
            updated[step.beat] = (updated[step.beat] ?? 0) + 1
            return updated
          })
        },
        setQueue,
        volume,
      )
      player.current = audioPlayer
      setTrack(result)
      setPhase('ready')
      await audioPlayer.start()
    } catch (cause) {
      if (context && context.state !== 'closed') await context.close()
      if (generation !== loadGeneration.current) return
      player.current?.destroy()
      planner.current?.free()
      player.current = undefined
      planner.current = undefined
      setTrack(undefined)
      setError(cause instanceof Error ? cause.message : String(cause))
      setPhase('error')
    }
  }, [volume])

  const seekToBeat = useCallback((beat: number) => {
    if (!track || !planner.current || !player.current || !Number.isFinite(beat)) return
    const target = Math.max(0, Math.min(track.analysis.beats.length - 1, Math.round(beat)))
    planner.current.continue_from(target)
    player.current.seek()
    setLive({ beat: target, jumpedFrom: null, probability: 1 })
  }, [track])

  const seek = useCallback((seconds: number) => {
    if (!track || !live || !planner.current) return
    const current = track.analysis.beats[live.beat]?.start ?? 0
    const targetTime = Math.max(0, current + seconds)
    let target = track.analysis.beats.findLastIndex((beat) => beat.start <= targetTime)
    target = Math.max(0, target)
    seekToBeat(target)
  }, [live, track, seekToBeat])

  const toggle = useCallback(async () => {
    await player.current?.toggle()
    setPaused(player.current?.paused ?? false)
  }, [])

  const changeVolume = useCallback((value: number) => {
    setVolume(value)
    player.current?.setVolume(value)
  }, [])

  useEffect(() => {
    if (!track) return
    const onKeyDown = (event: KeyboardEvent) => {
      const target = event.target
      if (target instanceof HTMLTextAreaElement || (target instanceof HTMLElement && target.isContentEditable)) return
      if (target instanceof HTMLInputElement && target.type !== 'range') return
      if (event.key === 'p') void toggle()
      else if (event.key === '0') seekToBeat(0)
      else if (event.key === 'b') seek(-10)
      else if (event.key === 'f') seek(10)
      else if (event.key === ',') changeVolume(Math.max(0, volume - .05))
      else if (event.key === '.') changeVolume(Math.min(1.5, volume + .05))
      else return
      event.preventDefault()
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [changeVolume, seek, seekToBeat, toggle, track, volume])

  if (!track) return <Landing dragging={dragging} error={error} fileName={fileName} phase={phase} onDragChange={setDragging} onFile={(file) => void load(file)} />

  const changeTrack = () => {
    loadGeneration.current++
    player.current?.destroy()
    planner.current?.free()
    player.current = undefined
    planner.current = undefined
    setTrack(undefined)
    setLive(undefined)
    setJumpCount(0)
    setPaused(false)
    sessionElapsed.current = 0
    sessionTick.current = 0
    setSessionSeconds(0)
    setPhase('idle')
  }

  return <Dashboard
    choices={choices} coverage={coverage} fileName={fileName} history={history}
    jumpCount={jumpCount} live={live} paused={paused} queue={queue} sessionSeconds={sessionSeconds}
    track={track} volume={volume} onChangeTrack={changeTrack} onSeek={seek} onSeekToBeat={seekToBeat}
    onToggle={() => void toggle()} onVolume={changeVolume}
  />
}

export default App
