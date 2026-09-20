import type { ReactNode } from 'react'
import type { PlannedBeat, PreparedTrack, TransitionProbability } from '../types'
import { beatLabel, formatSession, formatTime } from '../format'
import { Brand } from './Brand'
import { BeatConnections } from './BeatConnections'

interface DashboardProps {
  choices: TransitionProbability[]
  coverage: number[]
  fileName: string
  history: PlannedBeat[]
  jumpCount: number
  live?: PlannedBeat
  paused: boolean
  queue: PlannedBeat[]
  sessionSeconds: number
  track: PreparedTrack
  volume: number
  onChangeTrack: () => void
  onSeek: (seconds: number) => void
  onSeekToBeat: (beat: number) => void
  onToggle: () => void
  onVolume: (volume: number) => void
}

function Panel({ title, meta, className = '', children }: { title: string; meta?: string; className?: string; children: ReactNode }) {
  return <section className={`data-card ${className}`}><header><h2>{title}</h2>{meta && <span>{meta}</span>}</header>{children}</section>
}

export function Dashboard(props: DashboardProps) {
  const { choices, coverage, fileName, history, jumpCount, live, paused, queue, sessionSeconds, track, volume } = props
  const currentBeat = live?.beat ?? 0
  const currentTime = track.analysis.beats[currentBeat]?.start ?? 0
  const progress = currentBeat / Math.max(1, track.analysis.beats.length - 1) * 100
  const branchCount = track.graph.branches.reduce((sum, branches) => sum + branches.length, 0)
  const jumps = history.filter((step) => step.jumpedFrom !== null)
  const orderedChoices = [...choices].sort((left, right) => right.probability - left.probability).slice(0, 6)
  const coverageMaximum = Math.max(1, ...coverage)
  const coverageBins = Array.from({ length: 64 }, (_, index) => {
    const start = Math.floor(index * coverage.length / 64)
    const end = Math.max(start + 1, Math.floor((index + 1) * coverage.length / 64))
    return Math.max(0, ...coverage.slice(start, end)) / coverageMaximum
  })

  return <main className="app-page">
    <nav className="site-nav dashboard-nav"><Brand /><div><span className={`play-state ${paused ? 'is-paused' : ''}`}><i /> {paused ? 'Paused' : 'Playing'}</span><button className="ghost-button" onClick={props.onChangeTrack}>Change track</button></div></nav>

    <section className="track-heading">
      <div><span className="kicker">Infinite mix</span><h1 title={fileName}>{fileName}</h1><p>{track.analysis.tempo.toFixed(1)} BPM <i /> {track.analysis.beats.length} beats <i /> {branchCount} transitions</p></div>
      <div className="session-time"><small>Session</small><strong>{formatSession(sessionSeconds)}</strong></div>
    </section>

    <Panel title="Beat connections" meta={`${formatTime(currentTime)} / ${formatTime(track.analysis.duration)}`} className="position-card">
      <BeatConnections graph={track.graph} count={track.analysis.beats.length} currentBeat={currentBeat} queue={queue} />
      <div className="progress-track">
        <i style={{ width: `${progress}%` }} /><b style={{ left: `${progress}%` }} />
        <input className="track-seek" type="range" min="0" max={Math.max(0, track.analysis.beats.length - 1)} step="1"
          value={currentBeat} aria-label="Track position" aria-valuetext={`${formatTime(currentTime)} of ${formatTime(track.analysis.duration)}`}
          onChange={(event) => props.onSeekToBeat(event.target.valueAsNumber)} />
      </div>
      <div className="coverage-strip" aria-label="Playback coverage">
        {coverageBins.map((intensity, index) => <i
          key={index}
          title={`${Math.round(intensity * coverageMaximum)} visits`}
          style={{
            opacity: intensity ? .18 + Math.pow(intensity, 1.35) * .82 : .06,
            transform: `scaleY(${intensity ? .35 + intensity * .65 : .2})`,
          }}
        />)}
      </div>
      <div className="transport">
        <button onClick={() => props.onSeek(-10)} aria-label="Back 10 seconds">↶ <small>10</small></button>
        <button className="play-button" onClick={props.onToggle} aria-label={paused ? 'Play' : 'Pause'}>{paused ? '▶' : 'Ⅱ'}</button>
        <button onClick={() => props.onSeek(10)} aria-label="Forward 10 seconds"><small>10</small> ↷</button>
        <label className="volume-control"><span>VOL</span><input type="range" min="0" max="1.5" step="0.01" value={volume} onChange={(event) => props.onVolume(event.target.valueAsNumber)} /><strong>{Math.round(volume * 100)}%</strong></label>
      </div>
    </Panel>

    <Panel title="Live stitch" meta={`Beat ${beatLabel(currentBeat)}`} className="stitch-card">
      <div className="stitch-flow">
        <div className="past-beats">{history.slice(0, 4).reverse().map((step, index) => <span key={`${step.beat}-${index}`}>{beatLabel(step.beat)}<i className={step.jumpedFrom === null ? '' : 'jump'}>→</i></span>)}</div>
        <strong>{beatLabel(currentBeat)}</strong>
        <div className="future-beats">{queue.slice(1, 5).map((step, index) => <span key={`${step.beat}-${index}`}><i className={step.jumpedFrom === null ? '' : 'jump'}>→</i>{beatLabel(step.beat)}</span>)}</div>
      </div>
    </Panel>

    <div className="dashboard-grid">
      <Panel title="Recent jumps" meta={`${jumps.length} total`} className="jumps-card">
        <div className="jump-list">{jumps.slice(0, 6).map((step, index) => <div key={`${step.beat}-${index}`}><span className="jump-route"><b>{beatLabel(step.jumpedFrom ?? 0)}</b><i>→</i><b>{beatLabel(step.beat)}</b></span><span className="direction">{(step.jumpedFrom ?? 0) > step.beat ? 'backward' : 'forward'}</span><strong>{(step.probability * 100).toFixed(1)}%</strong></div>)}{!jumps.length && <p className="empty-state">The first jump will appear here.</p>}</div>
      </Panel>
      <Panel title="Next decision" meta="Adaptive odds" className="planner-card">
        <div className="odds-list">{orderedChoices.map((choice) => <div key={`${choice.destination}-${choice.is_sequential}`}><span>{choice.is_sequential ? 'Continue' : 'Jump'} <b>→ {beatLabel(choice.destination)}</b></span><i><u style={{ width: `${choice.probability * 100}%` }} /></i><strong>{(choice.probability * 100).toFixed(1)}%</strong></div>)}</div>
      </Panel>
    </div>

    <footer className="shortcut-bar"><span><kbd>B</kbd> back</span><span><kbd>F</kbd> forward</span><span><kbd>P</kbd> pause</span><span><kbd>,</kbd><kbd>.</kbd> volume</span><strong><i /> {jumpCount} jumps this session</strong></footer>
  </main>
}
