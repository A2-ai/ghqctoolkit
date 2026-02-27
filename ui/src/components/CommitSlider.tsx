import { Slider } from '@mantine/core'

interface CommitSliderProps {
  commits: Array<{ hash: string; file_changed: boolean }>
  value: number
  onChange: (idx: number) => void
  mb?: number
}

/**
 * A Mantine Slider that renders commit hashes as marks.
 *
 * Single-commit case: uses min=0, max=2, value=1 so Mantine positions the
 * single mark at exactly 50% of the track, matching the status-dot formula's
 * pct=0.5 above the slider.
 *
 * Multi-commit case: standard 0…N-1 slider.
 */
export function CommitSlider({ commits, value, onChange, mb = 40 }: CommitSliderProps) {
  if (commits.length === 1) {
    return (
      <Slider
        min={0}
        max={2}
        step={1}
        value={1}
        onChange={() => {}}
        marks={[{
          value: 1,
          label: (
            <span style={{ fontFamily: 'monospace', fontSize: 10, color: commits[0].file_changed ? '#111' : '#999' }}>
              {commits[0].hash.slice(0, 7)}
            </span>
          ),
        }]}
        label={null}
        mb={mb}
        styles={{ bar: { display: 'none' } }}
      />
    )
  }

  return (
    <Slider
      min={0}
      max={Math.max(0, commits.length - 1)}
      step={1}
      value={value}
      onChange={onChange}
      marks={commits.map((c, i) => ({
        value: i,
        label: (
          <span style={{ fontFamily: 'monospace', fontSize: 10, color: c.file_changed ? '#111' : '#999' }}>
            {c.hash.slice(0, 7)}
          </span>
        ),
      }))}
      label={null}
      mb={mb}
      styles={{ bar: { display: 'none' } }}
    />
  )
}
