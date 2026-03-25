import { useState, useEffect, useRef } from 'react'
import { api } from '../api'
import type { IndexStats } from '../types'
import { FileBrowser } from './FileBrowser'
import styles from './Landing.module.css'

interface LandingProps {
  onIndexed: (path: string, stats: IndexStats) => void
}

export function Landing({ onIndexed }: LandingProps) {
  const [path, setPath] = useState('')
  const [loading, setLoading] = useState(false)
  const [progress, setProgress] = useState(0)
  const [progressLabel, setProgressLabel] = useState('')
  const [stats, setStats] = useState<IndexStats | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [showBrowser, setShowBrowser] = useState(false)
  const [hidden, setHidden] = useState(false)
  const progressTimer = useRef<ReturnType<typeof setInterval> | null>(null)

  // Animate progress bar during indexing (indeterminate — we fake it)
  useEffect(() => {
    if (loading) {
      setProgress(0)
      setProgressLabel('Scanning files...')
      let p = 0
      progressTimer.current = setInterval(() => {
        p = Math.min(p + Math.random() * 4, 90)
        setProgress(p)
        if (p < 30) setProgressLabel('Scanning files...')
        else if (p < 60) setProgressLabel('Parsing symbols...')
        else if (p < 80) setProgressLabel('Building graph...')
        else setProgressLabel('Finalising index...')
      }, 200)
    } else {
      if (progressTimer.current) clearInterval(progressTimer.current)
    }
    return () => {
      if (progressTimer.current) clearInterval(progressTimer.current)
    }
  }, [loading])

  async function handleIndex() {
    if (!path.trim()) return
    setError(null)
    setStats(null)
    setLoading(true)

    try {
      const result = await api.index(path.trim())

      if ((result as any).cached) {
        // Already indexed — skip animation, go straight to explorer
        setLoading(false)
        setHidden(true)
        setTimeout(() => onIndexed(path.trim(), result), 300)
        return
      }

      setProgress(100)
      setProgressLabel('Done!')
      setStats(result)

      // Transition to explorer after 1.5s
      setTimeout(() => {
        setHidden(true)
        setTimeout(() => onIndexed(path.trim(), result), 600)
      }, 1500)
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
    } finally {
      setLoading(false)
    }
  }

  function handleKeyDown(e: React.KeyboardEvent) {
    if (e.key === 'Enter') handleIndex()
  }

  return (
    <>
      {showBrowser && (
        <FileBrowser
          initialPath={path || undefined}
          onSelect={(selected) => {
            setPath(selected)
            setShowBrowser(false)
          }}
          onClose={() => setShowBrowser(false)}
        />
      )}

      <div className={`${styles.landing}${hidden ? ' ' + styles.hidden : ''}`}>
        <div className={styles.bg}>
          <div className={`${styles.gear} ${styles.gear1}`} />
          <div className={`${styles.gear} ${styles.gear2}`} />
          <div className={`${styles.gear} ${styles.gear3}`} />
          <div className={`${styles.gear} ${styles.gear4}`} />
        </div>

        <div className={styles.logoContainer}>
          <div className={styles.logoRing} />
          <img src="/logo.png" alt="BearWisdom" />
        </div>

        <h1 className={styles.title}>BearWisdom</h1>

        <p className={styles.subtitle}>
          31 languages &middot; structural graph &middot; hybrid search
        </p>

        <div className={styles.form}>
          <div className={styles.pathInputWrapper}>
            <input
              className={styles.pathInput}
              type="text"
              placeholder="Enter path or browse..."
              value={path}
              onChange={(e) => setPath(e.target.value)}
              onKeyDown={handleKeyDown}
              disabled={loading}
            />
            <button
              className={styles.browseBtn}
              onClick={() => setShowBrowser(true)}
              title="Browse filesystem"
              disabled={loading}
            >
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z" />
              </svg>
            </button>
          </div>
          <button
            className={styles.indexBtn}
            onClick={handleIndex}
            disabled={loading || !path.trim()}
          >
            {loading ? 'Indexing...' : 'Index'}
          </button>
        </div>

        {error && <p className={styles.errorMsg}>{error}</p>}

        {(loading || stats) && (
          <div className={styles.progressContainer}>
            <div className={styles.progressLabel}>
              <span>{progressLabel}</span>
              <span>{Math.round(progress)}%</span>
            </div>
            <div className={styles.progressTrack}>
              <div className={styles.progressFill} style={{ width: `${progress}%` }} />
            </div>

            {stats && (
              <div className={styles.statsRow}>
                <div className={styles.statItem}>
                  <div className={styles.statValue}>{stats.file_count.toLocaleString()}</div>
                  <div className={styles.statLabel}>Files</div>
                </div>
                <div className={styles.statItem}>
                  <div className={styles.statValue}>{stats.symbol_count.toLocaleString()}</div>
                  <div className={styles.statLabel}>Symbols</div>
                </div>
                <div className={styles.statItem}>
                  <div className={styles.statValue}>{stats.edge_count.toLocaleString()}</div>
                  <div className={styles.statLabel}>Edges</div>
                </div>
                <div className={styles.statItem}>
                  <div className={styles.statValue}>{stats.duration_ms.toLocaleString()}</div>
                  <div className={styles.statLabel}>ms</div>
                </div>
              </div>
            )}
          </div>
        )}
      </div>
    </>
  )
}
