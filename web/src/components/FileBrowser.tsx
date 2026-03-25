import { useEffect, useState, useCallback } from 'react'
import { api } from '../api'
import styles from './FileBrowser.module.css'

interface FileBrowserProps {
  initialPath?: string
  onSelect: (path: string) => void
  onClose: () => void
}

function parseParts(path: string): string[] {
  if (!path) return []
  // Normalise to forward slashes, split, drop empties.
  const raw = path.replace(/\\/g, '/').split('/').filter(Boolean)
  return raw
}

function joinParts(parts: string[]): string {
  if (parts.length === 0) return ''
  // Detect Windows drive (e.g. "C:") vs Unix root
  const first = parts[0]
  if (/^[A-Za-z]:$/.test(first)) {
    if (parts.length === 1) return first + '\\'
    return first + '\\' + parts.slice(1).join('\\')
  }
  return '/' + parts.join('/')
}

export function FileBrowser({ initialPath, onSelect, onClose }: FileBrowserProps) {
  const [currentPath, setCurrentPath] = useState<string>(initialPath || '')
  const [dirs, setDirs] = useState<string[]>([])
  const [files, setFiles] = useState<string[]>([])
  const [loading, setLoading] = useState(false)
  const [selected, setSelected] = useState<string | null>(null)

  const navigate = useCallback(async (path: string) => {
    setLoading(true)
    setSelected(null)
    try {
      const result = await api.browse(path)
      setCurrentPath(path)
      setDirs(result.dirs)
      setFiles(result.files)
    } catch {
      setDirs([])
      setFiles([])
    } finally {
      setLoading(false)
    }
  }, [])

  useEffect(() => {
    navigate(initialPath || '')
  }, [])

  useEffect(() => {
    function onKeyDown(e: KeyboardEvent) {
      if (e.key === 'Escape') onClose()
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [onClose])

  const parts = parseParts(currentPath)

  function handleCrumb(index: number) {
    const sliced = parts.slice(0, index + 1)
    navigate(joinParts(sliced))
  }

  function fullChildPath(dirName: string): string {
    // Build the full path for a child directory entry.
    // The API returns bare names for subdirectories, but full paths for drives.
    if (!currentPath) {
      // At root — entries are drive paths like "C:\\"
      return dirName
    }
    // Append child to current path
    const sep = currentPath.includes('/') ? '/' : '\\'
    const base = currentPath.endsWith('\\') || currentPath.endsWith('/')
      ? currentPath
      : currentPath + sep
    return base + dirName
  }

  function handleDirClick(dir: string) {
    if (selected === dir) {
      // second click — enter directory
      navigate(fullChildPath(dir))
    } else {
      setSelected(dir)
    }
  }

  function handleDirDoubleClick(dir: string) {
    navigate(fullChildPath(dir))
  }

  function handleSelectFolder() {
    if (selected) {
      onSelect(fullChildPath(selected))
    } else if (currentPath) {
      onSelect(currentPath)
    }
  }

  const displayPath = selected ? fullChildPath(selected) : currentPath

  return (
    <div className={styles.overlay} onClick={(e) => { if (e.target === e.currentTarget) onClose() }}>
      <div className={styles.modal}>
        <div className={styles.header}>
          <div className={styles.headerTitle}>
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
              <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z" />
            </svg>
            Select Project Folder
          </div>
          <button className={styles.closeBtn} onClick={onClose}>&times;</button>
        </div>

        <div className={styles.breadcrumb}>
          <button className={styles.crumb} onClick={() => navigate('')}>
            Root
          </button>
          {parts.map((part, i) => {
            const isLast = i === parts.length - 1
            return (
              <span key={i} style={{ display: 'contents' }}>
                <span className={styles.sep}>/</span>
                {isLast ? (
                  <span className={styles.crumbCurrent}>{part}</span>
                ) : (
                  <button className={styles.crumb} onClick={() => handleCrumb(i)}>{part}</button>
                )}
              </span>
            )
          })}
        </div>

        <div className={styles.list}>
          {loading ? (
            <div className={styles.loading}>Loading...</div>
          ) : dirs.length === 0 && files.length === 0 ? (
            <div className={styles.empty}>Empty directory</div>
          ) : (
            <>
              {dirs.map((dir) => (
                <button
                  key={dir}
                  className={`${styles.item}${selected === dir ? ' ' + styles.selected : ''}`}
                  onClick={() => handleDirClick(dir)}
                  onDoubleClick={() => handleDirDoubleClick(dir)}
                >
                  <span className={styles.itemIcon}>
                    <svg width="16" height="16" viewBox="0 0 24 24" fill="currentColor">
                      <path d="M20 6h-8l-2-2H4c-1.1 0-2 .9-2 2v12c0 1.1.9 2 2 2h16c1.1 0 2-.9 2-2V8c0-1.1-.9-2-2-2z" />
                    </svg>
                  </span>
                  <span className={styles.itemName}>{
                    // Show the bare directory name. For drive roots like "C:\\" show "C:".
                    !currentPath
                      ? dir.replace(/[\\/]+$/, '')           // root level: strip trailing slashes
                      : dir                                   // subdirectory: already a bare name
                  }</span>
                </button>
              ))}
              {files.map((file) => (
                <div key={file} className={`${styles.item} ${styles.itemDimmed}`}>
                  <span className={styles.itemIcon} style={{ color: 'var(--text-faint)' }}>
                    <svg width="14" height="14" viewBox="0 0 24 24" fill="currentColor" opacity="0.5">
                      <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8l-6-6z" />
                    </svg>
                  </span>
                  <span className={styles.itemName}>{file.replace(/\\/g, '/').split('/').pop()}</span>
                </div>
              ))}
            </>
          )}
        </div>

        <div className={styles.footer}>
          <div className={styles.pathDisplay}>{displayPath || '/'}</div>
          <button className={styles.cancelBtn} onClick={onClose}>Cancel</button>
          <button className={styles.selectBtn} onClick={handleSelectFolder}>
            Select Folder
          </button>
        </div>
      </div>
    </div>
  )
}
