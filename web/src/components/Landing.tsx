import type { DragEvent } from 'react'
import { Brand } from './Brand'

type Phase = 'idle' | 'decoding' | 'analysing' | 'ready' | 'error'

interface LandingProps {
  dragging: boolean
  error: string
  fileName: string
  phase: Phase
  onDragChange: (dragging: boolean) => void
  onFile: (file: File) => void
}

const phaseCopy = {
  idle: ['Drop an audio file', 'or browse from your device'],
  decoding: ['Decoding your track…', 'Preparing audio locally'],
  analysing: ['Mapping musical connections…', 'Rust + WebAssembly at work'],
  ready: ['Track ready', 'Starting the infinite mix'],
  error: ['Try another audio file', 'or browse from your device'],
} as const

export function Landing({ dragging, error, fileName, phase, onDragChange, onFile }: LandingProps) {
  const handleDrop = (event: DragEvent<HTMLLabelElement>) => {
    event.preventDefault()
    onDragChange(false)
    const file = event.dataTransfer.files[0]
    if (file) onFile(file)
  }

  return <main className="landing-page">
    <nav className="site-nav"><Brand /><span className="local-pill"><i /> 100% local</span></nav>
    <section className="landing-grid">
      <div className="hero-copy">
        <span className="kicker">The infinite music machine</span>
        <h1>Keep the song.<br /><em>Lose the ending.</em></h1>
        <p>Eternal Jukebox finds musically compatible beats and weaves them into a mix that keeps evolving—all inside your browser.</p>
        <div className="feature-row"><span><i className="cyan" /> Private</span><span><i className="green" /> Instant playback</span><span><i className="mauve" /> No upload</span></div>
      </div>
      <div className="upload-card">
        <div className="mini-visual" aria-hidden="true">{Array.from({ length: 28 }, (_, index) => <i key={index} style={{ height: `${18 + ((index * 23) % 58)}%` }} />)}</div>
        <label
          className={`dropzone ${dragging ? 'is-dragging' : ''} ${phase === 'decoding' || phase === 'analysing' ? 'is-busy' : ''}`}
          onDragEnter={() => onDragChange(true)}
          onDragLeave={() => onDragChange(false)}
          onDragOver={(event) => event.preventDefault()}
          onDrop={handleDrop}
        >
          <input type="file" accept="audio/*" onChange={(event) => { const file = event.target.files?.[0]; if (file) onFile(file) }} />
          <span className="upload-icon">↥</span>
          <strong>{phaseCopy[phase][0]}</strong>
          <small>{fileName && phase !== 'idle' && phase !== 'error' ? fileName : phaseCopy[phase][1]}</small>
        </label>
        {error && <p className="upload-error" role="alert">{error}</p>}
        <p className="privacy-note"><span>◆</span> Audio never leaves this device</p>
      </div>
    </section>
    <footer className="landing-footer"><span>Rust-powered analysis</span><span>Web Audio playback</span><span>Adaptive infinite walk</span></footer>
  </main>
}
