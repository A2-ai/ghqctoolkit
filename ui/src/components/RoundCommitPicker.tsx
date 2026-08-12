// Round-aware commit picker, shared by the Notify (two handles), Review and
// Approve (one handle) panels of IssueDetailModal.
//
// `useRoundPicker` owns everything derived — the visible commit window, the
// draft-gap markers, handle snapping and the resolved from/to selection — and
// `RoundCommitPickerTrack` renders it. The panels keep owning the raw state
// (which index each handle is on, whether "Show all commits" is checked) so that
// their existing reset-on-issue-change effects keep working unchanged.

import { useEffect, useMemo, useRef, useState, type ReactNode } from 'react'
import { Checkbox, ScrollArea, Slider, Text, UnstyledButton } from '@mantine/core'
import type { RoundInfo } from '~/api/rounds'
import { CommitSlider } from '~/components/CommitSlider'
import { draftGapRuns, openRoundWindow, type OrderedCommit, type RoundWindow } from '~/utils/rounds'

// Commit status dot colors (rendered oldest→newest, lowest→highest)
export const STATUS_DOT_COLORS: Record<string, string> = {
  initial:      '#339af0', // blue
  notification: '#ffd43b', // yellow
  approved:     '#51cf66', // green
  reviewed:     '#ff922b', // orange
}
export const STATUS_ORDER = ['initial', 'notification', 'approved', 'reviewed'] as const

export interface PickerCommit extends OrderedCommit {
  /** Index into the oldest-first `orderedCommits` list. */
  origIdx: number
}

interface GapMarker {
  key: string
  /** Visible-slot index this marker sits in front of. */
  beforeSlot: number
  indices: number[]
  expanded: boolean
}

export interface RoundPicker {
  /** Commits offered on the track, oldest-first. */
  visibleCommits: PickerCommit[]
  /** Collapsed draft-gap runs, only ever non-empty in full-history mode. */
  gapMarkers: GapMarker[]
  toggleGap: (key: string) => void
  /** The open round's span, when one could be resolved. */
  window: RoundWindow | null
  /** True when the track is narrowed to the open round rather than full history. */
  scoped: boolean
  /** Slider positions (indices into `visibleCommits`). */
  snapA: number
  snapB: number
  /** Resolved selection. In single mode `from` is the comparison baseline. */
  fromCommit: PickerCommit | undefined
  toCommit: PickerCommit | undefined
  /** Alias of `toCommit`, for the single-handle panels. */
  selectedCommit: PickerCommit | undefined
  /** Number of commits spanned by the selection (0 when from === to). */
  spannedCount: number
  /** Horizontal-scroll viewport ref, so the track opens scrolled to the newest commit. */
  viewportRef: React.RefObject<HTMLDivElement | null>
  mode: 'single' | 'range'
  onA: (visibleIdx: number) => void
  onB: (visibleIdx: number) => void
}

export interface UseRoundPickerOptions {
  /** Oldest-first commit list. */
  orderedCommits: OrderedCommit[]
  rounds: RoundInfo[]
  openRoundIndex: number | null
  mode: 'single' | 'range'
  /** Single mode: the selected origIdx. Range mode: handle A's origIdx. */
  a: number
  setA: (origIdx: number) => void
  /** Range mode only: handle B's origIdx. */
  b?: number
  setB?: (origIdx: number) => void
  /**
   * Extra origIdxs pinned visible regardless of the round window or the
   * file_changed / statuses emphasis filter — the panels' existing
   * `exceptionIdx`, plus anything the defaults resolved to outside the window
   * (e.g. the previous round's approval as a notify from-commit).
   */
  forcedIdxs?: number[]
  showAll: boolean
}

export function useRoundPicker(opts: UseRoundPickerOptions): RoundPicker {
  const { orderedCommits, rounds, openRoundIndex, mode, a, setA, b, setB, showAll } = opts

  const window_ = useMemo(
    () => openRoundWindow(orderedCommits, rounds, openRoundIndex),
    [orderedCommits, rounds, openRoundIndex],
  )

  const forced = useMemo(() => {
    const s = new Set<number>((opts.forcedIdxs ?? []).filter((i) => i >= 0))
    s.add(a)
    if (mode === 'range' && b !== undefined) s.add(b)
    return s
  }, [opts.forcedIdxs, a, b, mode])

  // Scoping only narrows anything when the window is smaller than the history.
  // For a legacy single `Initial QC` round the window is the whole history, so
  // this is a no-op and those issues behave exactly as before.
  const scoped =
    !showAll &&
    window_ !== null &&
    (window_.anchorIdx > 0 || window_.endIdx < orderedCommits.length - 1)

  const gapRuns = useMemo(
    () => (showAll ? draftGapRuns(orderedCommits, rounds) : []),
    [showAll, orderedCommits, rounds],
  )

  const [expandedGaps, setExpandedGaps] = useState<Set<string>>(new Set())
  const gapKey = (run: number[]) => `gap-${run[0]}-${run[run.length - 1]}`
  const toggleGap = (key: string) =>
    setExpandedGaps((prev) => {
      const next = new Set(prev)
      if (next.has(key)) next.delete(key)
      else next.add(key)
      return next
    })

  // Indices hidden behind a collapsed draft-gap marker.
  const collapsedIdxs = useMemo(() => {
    const s = new Set<number>()
    for (const run of gapRuns) {
      if (expandedGaps.has(gapKey(run))) continue
      for (const i of run) if (!forced.has(i)) s.add(i)
    }
    return s
  }, [gapRuns, expandedGaps, forced])

  const visibleCommits = useMemo(
    () =>
      orderedCommits
        .map((c, i) => ({ ...c, origIdx: i }))
        .filter(({ file_changed, statuses, origIdx }) => {
          if (forced.has(origIdx)) return true
          if (collapsedIdxs.has(origIdx)) return false
          if (showAll) return true
          if (window_ && (origIdx < window_.anchorIdx || origIdx > window_.endIdx)) return false
          return file_changed || statuses.length > 0
        }),
    [orderedCommits, forced, collapsedIdxs, showAll, window_],
  )

  const gapMarkers = useMemo<GapMarker[]>(
    () =>
      gapRuns
        .map((run) => {
          const key = gapKey(run)
          const expanded = expandedGaps.has(key)
          const beforeSlot = visibleCommits.filter((c) => c.origIdx < run[0]).length
          return { key, beforeSlot, indices: run, expanded }
        })
        // A run made entirely of forced (pinned) commits has nothing to collapse.
        .filter((m) => m.indices.some((i) => !opts.forcedIdxs?.includes(i))),
    [gapRuns, expandedGaps, visibleCommits, opts.forcedIdxs],
  )

  const viewportRef = useRef<HTMLDivElement>(null)
  const hasScrolledToRight = useRef(false)
  useEffect(() => {
    if (!hasScrolledToRight.current && visibleCommits.length > 0 && viewportRef.current) {
      viewportRef.current.scrollLeft = viewportRef.current.scrollWidth
      hasScrolledToRight.current = true
    }
  }, [visibleCommits.length])

  const snapToVisible = (targetOrigIdx: number): number => {
    const exact = visibleCommits.findIndex((c) => c.origIdx === targetOrigIdx)
    if (exact >= 0) return exact
    let best = 0
    let bestDist = Infinity
    for (let i = 0; i < visibleCommits.length; i++) {
      const dist = Math.abs(visibleCommits[i].origIdx - targetOrigIdx)
      if (dist < bestDist) {
        bestDist = dist
        best = i
      }
    }
    return best
  }

  const snapA = snapToVisible(a)
  const snapB = mode === 'range' && b !== undefined ? snapToVisible(b) : snapA

  let fromCommit: PickerCommit | undefined
  let toCommit: PickerCommit | undefined
  if (mode === 'range') {
    // Either handle may be dragged past the other; from = earlier, to = later.
    fromCommit = visibleCommits[Math.min(snapA, snapB)]
    toCommit = visibleCommits[Math.max(snapA, snapB)]
  } else {
    toCommit = visibleCommits[snapA]
    // The comparison baseline for a single-handle picker is the round anchor
    // (what the round's work is measured against), falling back to the oldest
    // commit on the track.
    const baseIdx = window_ ? window_.anchorIdx : (visibleCommits[0]?.origIdx ?? 0)
    fromCommit =
      visibleCommits.find((c) => c.origIdx === baseIdx) ??
      visibleCommits.filter((c) => c.origIdx <= (toCommit?.origIdx ?? 0))[0] ??
      visibleCommits[0]
  }

  const spannedCount =
    fromCommit && toCommit ? Math.max(0, toCommit.origIdx - fromCommit.origIdx) : 0

  return {
    visibleCommits,
    gapMarkers,
    toggleGap,
    window: window_,
    scoped,
    snapA,
    snapB,
    fromCommit,
    toCommit,
    selectedCommit: mode === 'single' ? toCommit : undefined,
    spannedCount,
    viewportRef,
    mode,
    onA: (visibleIdx) => setA(visibleCommits[visibleIdx]?.origIdx ?? a),
    onB: (visibleIdx) => setB?.(visibleCommits[visibleIdx]?.origIdx ?? b ?? a),
  }
}

// ---------------------------------------------------------------------------
// Presentation
// ---------------------------------------------------------------------------

interface TrackProps {
  title: string
  picker: RoundPicker
  showAll: boolean
  onShowAllChange: (value: boolean) => void
  /** Rendered between the legend and the receipt (From/To blocks, checkboxes). */
  children?: ReactNode
  /** Extra content rendered above the track, e.g. the empty-diff notice. */
  banner?: ReactNode
  testId?: string
}

/** Positions a slot boundary using the same percentage formula as the status dots. */
function slotBoundaryLeft(slot: number, n: number): string {
  if (n <= 1) return 'calc(10px + 50%)'
  const pct = Math.min(1, Math.max(0, (slot - 0.5) / (n - 1)))
  return `calc(10px + ${pct * 100}% - ${pct * 20}px)`
}

export function RoundCommitPickerTrack({
  title,
  picker,
  showAll,
  onShowAllChange,
  children,
  banner,
  testId,
}: TrackProps) {
  const { visibleCommits, gapMarkers, toggleGap, viewportRef } = picker
  const n = visibleCommits.length

  return (
    <>
      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
        <div style={{ display: 'flex', alignItems: 'baseline', gap: 8, minWidth: 0 }}>
          <Text size="sm" fw={700}>{title}</Text>
          {picker.scoped && picker.window && (
            <Text size="xs" c="dimmed" data-testid="picker-scope">
              scoped to {picker.window.round.name}
            </Text>
          )}
        </div>
        <Checkbox
          label="Show all commits"
          checked={showAll}
          onChange={(e) => onShowAllChange(e.currentTarget.checked)}
        />
      </div>

      {banner}

      <ScrollArea
        scrollbars="x"
        type="always"
        offsetScrollbars
        viewportRef={viewportRef}
        style={{ marginLeft: -16, marginRight: -16 }}
      >
        <div
          style={{
            minWidth: Math.max(300, n * 60 + 56),
            display: 'flex',
            flexDirection: 'column',
            gap: 4,
            paddingLeft: 16,
            paddingRight: 40,
          }}
          data-testid={testId}
        >
          {gapMarkers.length > 0 && (
            <div style={{ position: 'relative', height: 20 }} data-testid="draft-gap-row">
              {gapMarkers.map((m) => (
                <UnstyledButton
                  key={m.key}
                  onClick={() => toggleGap(m.key)}
                  data-testid={`draft-gap-${m.indices[0]}`}
                  title={
                    m.expanded
                      ? 'Hide these commits — they belong to no round'
                      : 'These commits belong to no round (edited between rounds)'
                  }
                  style={{
                    position: 'absolute',
                    left: slotBoundaryLeft(m.beforeSlot, n),
                    transform: 'translateX(-50%)',
                    whiteSpace: 'nowrap',
                    fontSize: 10,
                    lineHeight: '16px',
                    padding: '0 6px',
                    borderRadius: 8,
                    border: '1px dashed var(--mantine-color-gray-5)',
                    color: 'var(--mantine-color-dimmed)',
                    backgroundColor: 'var(--mantine-color-body)',
                  }}
                >
                  {m.expanded
                    ? `⋯ hide ${m.indices.length} ⋯`
                    : `⋯ ${m.indices.length} commit${m.indices.length === 1 ? '' : 's'} ⋯`}
                </UnstyledButton>
              ))}
            </div>
          )}

          <div style={{ position: 'relative', height: 8 }}>
            {visibleCommits.map((c, i) => {
              const pct = n > 1 ? i / (n - 1) : 0.5
              const left = `calc(10px + ${pct * 100}% - ${pct * 20}px)`
              return (
                <div
                  key={i}
                  style={{ position: 'absolute', left, transform: 'translateX(-50%)', display: 'flex', gap: 2 }}
                >
                  {STATUS_ORDER.filter((s) => c.statuses.includes(s)).map((s) => (
                    <span
                      key={s}
                      title={s}
                      style={{
                        display: 'inline-block',
                        width: 7,
                        height: 7,
                        borderRadius: '50%',
                        backgroundColor: STATUS_DOT_COLORS[s],
                      }}
                    />
                  ))}
                </div>
              )
            })}
          </div>

          <div style={{ position: 'relative' }}>
            <CommitSlider
              commits={visibleCommits}
              value={picker.snapA}
              mb={28}
              onChange={picker.onA}
            />
            {picker.mode === 'range' && n > 1 && (
              <div style={{ position: 'absolute', top: 0, left: 0, right: 0 }}>
                <Slider
                  min={0}
                  max={Math.max(0, n - 1)}
                  step={1}
                  value={picker.snapB}
                  onChange={picker.onB}
                  label={null}
                  styles={{
                    bar: { display: 'none' },
                    root: { pointerEvents: 'none' },
                    thumb: { pointerEvents: 'auto', zIndex: 3 },
                    track: { backgroundColor: 'transparent' },
                    mark: { display: 'none' },
                    markLabel: { display: 'none' },
                  }}
                />
              </div>
            )}
          </div>
        </div>
      </ScrollArea>

      <div style={{ display: 'flex', gap: 14, flexWrap: 'wrap', justifyContent: 'center' }}>
        {STATUS_ORDER.map((s) => (
          <div key={s} style={{ display: 'flex', alignItems: 'center', gap: 4 }}>
            <span
              style={{
                display: 'inline-block',
                width: 8,
                height: 8,
                borderRadius: '50%',
                backgroundColor: STATUS_DOT_COLORS[s],
              }}
            />
            <Text size="xs" c="dimmed" style={{ textTransform: 'capitalize' }}>{s}</Text>
          </div>
        ))}
      </div>

      {children}

      <ComparisonReceipt picker={picker} />
    </>
  )
}

/**
 * S7: a one-line receipt of the selected comparison.
 *
 * The commit count is derived from the ordered commit list. There is no backend
 * endpoint exposing per-range line stats, so no `+x/−y` is shown.
 */
export function ComparisonReceipt({ picker }: { picker: RoundPicker }) {
  const { fromCommit, toCommit, spannedCount } = picker
  if (!toCommit) return null
  const from = fromCommit?.hash.slice(0, 7) ?? '—'
  const to = toCommit.hash.slice(0, 7)
  return (
    <Text
      size="xs"
      c="dimmed"
      ta="center"
      data-testid="comparison-receipt"
      style={{ fontVariantNumeric: 'tabular-nums' }}
    >
      <span style={{ fontFamily: 'monospace' }}>{from}</span>
      {' → '}
      <span style={{ fontFamily: 'monospace' }}>{to}</span>
      {' · '}
      {spannedCount} commit{spannedCount === 1 ? '' : 's'}
    </Text>
  )
}
