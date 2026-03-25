import styles from './ZoomControls.module.css'

interface ZoomControlsProps {
  onZoomIn: () => void
  onZoomOut: () => void
  onZoomReset: () => void
}

export function ZoomControls({ onZoomIn, onZoomOut, onZoomReset }: ZoomControlsProps) {
  return (
    <div className={styles.zoomControls}>
      <button className={styles.zoomBtn} onClick={onZoomIn} title="Zoom in">
        +
      </button>
      <button className={styles.zoomBtn} onClick={onZoomOut} title="Zoom out">
        &minus;
      </button>
      <button className={styles.zoomBtn} onClick={onZoomReset} title="Reset zoom">
        &otimes;
      </button>
    </div>
  )
}
