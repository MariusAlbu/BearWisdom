export function GraphLegend() {
  return (
    <div
      style={{
        position: 'absolute',
        bottom: 40,
        left: 12,
        width: 150,
        background: '#242019',
        border: '1px solid #3D3226',
        borderRadius: 8,
        padding: '10px 14px',
        fontSize: 11,
        zIndex: 10,
        opacity: 0.95,
        boxShadow: '0 4px 12px rgba(0,0,0,0.5)',
      }}
    >
      <div
        style={{
          fontWeight: 700,
          fontSize: 10,
          textTransform: 'uppercase',
          letterSpacing: 1,
          color: '#8B7D6B',
          marginBottom: 8,
        }}
      >
        Legend
      </div>
      <div style={{ marginBottom: 8 }}>
        <div
          style={{
            fontSize: 9,
            textTransform: 'uppercase',
            letterSpacing: 1,
            color: '#5C5044',
            marginBottom: 4,
          }}
        >
          Nodes
        </div>
        {[
          { label: 'Class', color: '#58a6ff' },
          { label: 'Interface', color: '#bc8cff' },
          { label: 'Method', color: '#3fb950' },
          { label: 'Enum', color: '#d29922' },
        ].map(({ label, color }) => (
          <div
            key={label}
            style={{
              display: 'flex',
              alignItems: 'center',
              gap: 8,
              marginBottom: 3,
              color: '#B8A892',
            }}
          >
            <svg width="10" height="10" style={{ flexShrink: 0, display: 'block' }}>
              <circle cx="5" cy="5" r="5" fill={color} fillOpacity={0.85} />
            </svg>
            <span>{label}</span>
          </div>
        ))}
      </div>
      <div>
        <div
          style={{
            fontSize: 9,
            textTransform: 'uppercase',
            letterSpacing: 1,
            color: '#5C5044',
            marginBottom: 4,
          }}
        >
          Edges
        </div>
        {[
          { label: 'Calls', stroke: '#b0b8c1', dash: undefined, w: 2, op: 1 },
          { label: 'Inherits', stroke: '#79bfff', dash: undefined, w: 3, op: 1 },
          { label: 'Implements', stroke: '#d0a8ff', dash: '5 3', w: 2, op: 1 },
          { label: 'Type ref', stroke: '#8b949e', dash: '3 3', w: 1.2, op: 0.6 },
        ].map(({ label, stroke, dash, w, op }) => (
          <div
            key={label}
            style={{
              display: 'flex',
              alignItems: 'center',
              gap: 8,
              marginBottom: 3,
              color: '#B8A892',
            }}
          >
            <svg width="20" height="6" style={{ flexShrink: 0, display: 'block' }}>
              <line
                x1="0"
                y1="3"
                x2="20"
                y2="3"
                stroke={stroke}
                strokeWidth={w}
                strokeDasharray={dash}
                strokeOpacity={op}
              />
            </svg>
            <span>{label}</span>
          </div>
        ))}
      </div>
    </div>
  )
}
