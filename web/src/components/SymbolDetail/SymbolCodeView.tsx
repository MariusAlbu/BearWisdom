import styles from './SymbolDetail.module.css'

interface SymbolCodeViewProps {
  content: string | null
  loading: boolean
  startLine?: number
  endLine?: number
}

export function SymbolCodeView({ content, loading, startLine, endLine }: SymbolCodeViewProps) {
  return (
    <>
      {startLine !== undefined && endLine !== undefined && (
        <div className={styles.codeLineRange}>
          Lines {startLine}–{endLine}
        </div>
      )}
      {loading ? (
        <div className={styles.detailBody}>
          <div className={styles.loadingText}>Loading code...</div>
        </div>
      ) : content !== null ? (
        <pre className={styles.codePreview}>{content}</pre>
      ) : (
        <div className={styles.detailBody}>
          <div className={styles.loadingText}>Could not load source file.</div>
        </div>
      )}
    </>
  )
}
