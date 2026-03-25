import headerStyles from './Header.module.css'

interface EmbedButtonProps {
  state: 'idle' | 'running' | 'done' | 'error'
  count: number
  error: string | null
  onEmbed: () => void
}

export function EmbedButton({ state, count, error, onEmbed }: EmbedButtonProps) {
  if (state === 'idle') {
    return (
      <button
        className={headerStyles.embedBtn}
        onClick={onEmbed}
        title="Compute embeddings to enable AI Search"
      >
        Enable AI Search
      </button>
    )
  }

  if (state === 'running') {
    return (
      <span
        className={headerStyles.embedRunning}
        title="Computing vector embeddings for all code chunks. This may take several minutes."
      >
        <span
          className={headerStyles.searchSpinner}
          style={{ position: 'static', transform: 'none' }}
        />
        Embedding (may take a few minutes)...
      </span>
    )
  }

  if (state === 'done') {
    return (
      <span
        className={headerStyles.embedDone}
        title={`${count.toLocaleString()} chunks embedded`}
      >
        AI Ready
      </span>
    )
  }

  // error
  return (
    <button
      className={headerStyles.embedError}
      onClick={onEmbed}
      title={error ?? 'Click to retry'}
    >
      Embed failed
    </button>
  )
}

