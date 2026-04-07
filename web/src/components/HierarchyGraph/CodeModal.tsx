import { useEffect, useRef } from 'react'
import styles from './CodeModal.module.css'

interface CodeModalProps {
  filePath: string
  content: string | null
  loading: boolean
  error: string | null
  onClose: () => void
}

export function CodeModal({ filePath, content, loading, error, onClose }: CodeModalProps) {
  const overlayRef = useRef<HTMLDivElement>(null)

  // Close on Escape
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if (e.key === 'Escape') onClose()
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [onClose])

  // Close on backdrop click
  function handleOverlayClick(e: React.MouseEvent<HTMLDivElement>) {
    if (e.target === overlayRef.current) onClose()
  }

  // Short display name: last two path segments
  const displayPath = filePath.replace(/\\/g, '/').split('/').slice(-2).join('/')

  return (
    <div
      ref={overlayRef}
      className={styles.overlay}
      onClick={handleOverlayClick}
      role="dialog"
      aria-modal="true"
      aria-label={`Code preview: ${displayPath}`}
    >
      <div className={styles.modal}>
        {/* Header */}
        <div className={styles.header}>
          <span className="material-symbols-outlined" style={{ fontSize: '14px', color: '#3fb950' }}>
            code
          </span>
          <span className={styles.filePath} title={filePath}>
            {displayPath}
          </span>
          <button className={styles.closeBtn} onClick={onClose} aria-label="Close">
            ×
          </button>
        </div>

        {/* Body */}
        <div className={styles.body}>
          {loading && (
            <div className={styles.stateMsg}>
              <div className={styles.spinner} />
              Loading…
            </div>
          )}
          {error && (
            <div className={styles.errorMsg}>
              <span className="material-symbols-outlined">error</span>
              {error}
            </div>
          )}
          {content && !loading && (
            <table className={styles.codeTable} aria-label="Source code">
              <tbody>
                {content.split('\n').map((line, i) => (
                  <tr key={i} id={`cm-line-${i + 1}`} className={styles.codeLine}>
                    <td className={styles.lineNumber}>{i + 1}</td>
                    <td className={styles.lineContent}>{line || ' '}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </div>
      </div>
    </div>
  )
}
