interface Props {
  onClick: () => void
  disabled?: boolean
}

export function AddFileCard({ onClick, disabled = false }: Props) {
  return (
    <div
      onClick={disabled ? undefined : onClick}
      style={{
        height: '100%',
        borderRadius: 6,
        border: `2px dashed ${disabled ? '#ced4da' : '#74c69d'}`,
        backgroundColor: disabled ? '#f8f9fa' : '#f0faf4',
        display: 'flex',
        flexDirection: 'column',
        alignItems: 'center',
        justifyContent: 'center',
        gap: 6,
        cursor: disabled ? 'default' : 'pointer',
        boxSizing: 'border-box',
        color: disabled ? '#adb5bd' : '#2f7a3b',
        transition: 'background-color 0.15s, border-color 0.15s',
        opacity: disabled ? 0.7 : 1,
      }}
      onMouseEnter={disabled ? undefined : (e => {
        const el = e.currentTarget
        el.style.backgroundColor = '#d3f0df'
        el.style.borderColor = '#2f7a3b'
      })}
      onMouseLeave={disabled ? undefined : (e => {
        const el = e.currentTarget
        el.style.backgroundColor = '#f0faf4'
        el.style.borderColor = '#74c69d'
      })}
    >
      <span style={{ fontSize: 36, lineHeight: 1, fontWeight: 300 }}>+</span>
      <span style={{ fontSize: 12, fontWeight: 600, letterSpacing: 0.2 }}>Create New QC</span>
      {disabled && (
        <span style={{ fontSize: 11, color: '#adb5bd', textAlign: 'center', padding: '0 8px' }}>
          Select a milestone first
        </span>
      )}
    </div>
  )
}
