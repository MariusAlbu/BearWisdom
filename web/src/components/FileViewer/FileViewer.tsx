import styles from './FileViewer.module.css'

interface FileViewerProps {
  filePath: string
  line?: number
  content: string | null
  loading: boolean
  onClose: () => void
}

export function FileViewer({ filePath, line, content, loading, onClose }: FileViewerProps) {
  return (
    <div className={styles.fileViewer}>
      <div className={styles.fileViewerHeader}>
        <span className={styles.fileViewerPath}>{filePath}</span>
        {line !== undefined && (
          <span className={styles.fileViewerLine}>Line {line}</span>
        )}
        <button className={styles.fileViewerClose} onClick={onClose}>
          &times;
        </button>
      </div>
      <div className={styles.fileViewerContent}>
        {loading ? (
          <div className={styles.fileViewerLoading}>Loading file...</div>
        ) : (
          <pre className={styles.fileViewerCode}>
            {content?.split('\n').map((lineText, i) => {
              const lineNum = i + 1
              const isTarget = line === lineNum
              return (
                <div
                  key={i}
                  id={`file-line-${lineNum}`}
                  className={`${styles.codeLine}${isTarget ? ' ' + styles.codeLineHighlight : ''}`}
                >
                  <span className={styles.lineNumber}>{lineNum}</span>
                  <span className={styles.lineContent}>{lineText}</span>
                </div>
              )
            })}
          </pre>
        )}
      </div>
    </div>
  )
}
