import type { CSSProperties } from 'react'

interface ToggleFieldProps {
  label: string
  checked: boolean
  onChange: (checked: boolean) => void
  disabled?: boolean
  color?: string
  labelStyle?: CSSProperties
  rootStyle?: CSSProperties
}

const INPUT_STYLE: CSSProperties = {
  position: 'absolute',
  inset: 0,
  width: '100%',
  height: '100%',
  margin: 0,
  opacity: 0,
  cursor: 'inherit',
  zIndex: 1,
}

export function ToggleField({
  label,
  checked,
  onChange,
  disabled = false,
  color = '#2f7a3b',
  labelStyle,
  rootStyle,
}: ToggleFieldProps) {
  return (
    <label
      style={{
        position: 'relative',
        display: 'inline-flex',
        alignItems: 'center',
        gap: 8,
        cursor: disabled ? 'not-allowed' : 'pointer',
        opacity: disabled ? 0.6 : 1,
        userSelect: 'none',
        ...rootStyle,
      }}
    >
      <input
        type="checkbox"
        role="switch"
        checked={checked}
        disabled={disabled}
        onChange={(event) => onChange(event.currentTarget.checked)}
        style={INPUT_STYLE}
      />
      <span
        aria-hidden="true"
        style={{
          width: 28,
          height: 16,
          borderRadius: 999,
          border: `1px solid ${checked ? color : '#adb5bd'}`,
          backgroundColor: checked ? color : '#e9ecef',
          position: 'relative',
          flexShrink: 0,
          pointerEvents: 'none',
          transition: 'background-color 120ms ease, border-color 120ms ease',
        }}
      >
        <span
          style={{
            position: 'absolute',
            top: 1,
            left: checked ? 13 : 1,
            width: 12,
            height: 12,
            borderRadius: '50%',
            backgroundColor: 'white',
            pointerEvents: 'none',
            transition: 'left 120ms ease',
          }}
        />
      </span>
      <span style={{ fontSize: 12, color: '#1f2933', pointerEvents: 'none', ...labelStyle }}>{label}</span>
    </label>
  )
}
