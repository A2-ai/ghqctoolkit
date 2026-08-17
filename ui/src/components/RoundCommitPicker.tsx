// Segment-aware commit picker, shared by the Notify (two handles), Review and
// Approve (one handle) panels of IssueDetailModal.
//
// U1: the track is rendered segment by segment from `segments`. Nothing in here
// computes round windows, coverage or gap runs — the API already owns which commit
// belongs to which segment (D7), so scoping is a read of `segIdx`, the collapsible
// gaps *are* the Gap segments, and a Gap whose bounds are not ancestrally connected
// puts a visible break in the track instead of letting a continuous slider imply a
// linear path that does not exist.
//
// `useRoundPicker` owns everything derived from that — the visible commit window,
// the gap markers, the breaks, handle snapping and the resolved from/to selection —
// and `RoundCommitPickerTrack` renders it. The panels keep owning the raw state
// (which index each handle is on, whether "Show all commits" is checked) so that
// their existing reset-on-issue-change effects keep working unchanged.

import { useEffect, useMemo, useRef, useState, type ReactNode } from 'react'
import { Checkbox, ScrollArea, Slider, Text, UnstyledButton } from '@mantine/core'
import type { RoundSegment, Segment, UnplaceableReason } from '~/api/rounds'
import { CommitSlider } from '~/components/CommitSlider'
import {
  activeRound,
  activeRoundPos,
  unplaceableReasonText,
  type OrderedCommit,
} from '~/utils/rounds'

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

/**
 * A discontinuity in the track: the two ends of a Gap segment are not ancestrally
 * connected, so the commits either side of it are not on one path.
 */
interface TrackBreak {
  /** Position in `segments` of the Gap that is not continuous. */
  segIdx: number
  /** Visible-slot index this break sits in front of. */
  beforeSlot: number
  continuity: 'diverged' | 'unrelated'
  /** The commit the two histories share; null when they share none. */
  mergeBase: string | null
}

export interface RoundPicker {
  /** Commits offered on the track, oldest-first. */
  visibleCommits: PickerCommit[]
  /** Collapsed Gap segments, only ever non-empty in full-history mode. */
  gapMarkers: GapMarker[]
  toggleGap: (key: string) => void
  /** Breaks the track must draw, oldest-first. Empty when every Gap is linear. */
  breaks: TrackBreak[]
  /** The round the track is narrowed to, when one is open and narrowing it does anything. */
  scopeRound: RoundSegment | null
  /** True when the track is narrowed to the open round rather than full history. */
  scoped: boolean
  /**
   * Why the open round could not be scoped to (D4/S4). An `Unplaceable` round owns no
   * commits, so narrowing to it would leave a track of forced defaults labelled with
   * that round's name — commits from *other* segments presented as if they were its
   * own. Non-null means the track fell back to full history and must say why.
   */
  scopeUnplaceableReason: UnplaceableReason | null
  /** Slider positions (indices into `visibleCommits`). */
  snapA: number
  snapB: number
  /** Resolved selection. In single mode `from` is the comparison baseline. */
  fromCommit: PickerCommit | undefined
  toCommit: PickerCommit | undefined
  /** Alias of `toCommit`, for the single-handle panels. */
  selectedCommit: PickerCommit | undefined
  /**
   * U1: the selection spans a break, so `fromCommit` is *not* connected to
   * `toCommit`. Surfaces render the from-handle detached rather than as a point on
   * a continuous track.
   */
  detached: boolean
  /** Number of commits spanned by the selection (0 when from === to). */
  spannedCount: number
  /** Horizontal-scroll viewport ref, so the track opens scrolled to the newest commit. */
  viewportRef: React.RefObject<HTMLDivElement | null>
  mode: 'single' | 'range'
  onA: (visibleIdx: number) => void
  onB: (visibleIdx: number) => void
}

export interface UseRoundPickerOptions {
  /** Oldest-first commit list, from `flattenSegmentCommits(segments)`. */
  orderedCommits: OrderedCommit[]
  /** The thread's segments, oldest-first, exactly as the API returned them. */
  segments: Segment[]
  mode: 'single' | 'range'
  /** Single mode: the selected origIdx. Range mode: handle A's origIdx. */
  a: number
  setA: (origIdx: number) => void
  /** Range mode only: handle B's origIdx. */
  b?: number
  setB?: (origIdx: number) => void
  /**
   * Extra origIdxs pinned visible regardless of the open round's scope or the
   * file_changed / statuses emphasis filter — the panels' existing `exceptionIdx`,
   * plus anything the defaults resolved to outside the open round (e.g. the previous
   * round's approval as a notify from-commit).
   */
  forcedIdxs?: number[]
  showAll: boolean
}

export function useRoundPicker(opts: UseRoundPickerOptions): RoundPicker {
  const { orderedCommits, segments, mode, a, setA, b, setB, showAll } = opts

  // Scope is the open round's own commit ownership — no window arithmetic. The
  // round's `commits` already include its `opened_at` (W2), which is the commit its
  // work is measured against, so the anchor stays on the track exactly as before.
  const scopeRound = useMemo(() => activeRound(segments), [segments])
  const openRoundPos = useMemo(() => activeRoundPos(segments), [segments])

  // S4/D4: an unplaceable round owns no commits, so its scope is empty. Narrowing to
  // it collapses the track to whatever the defaults forced visible — for a round 2
  // whose branch is gone, a round 1 commit under the label "scoped to Round 2". The
  // rail already explains the placement failure; the picker must not quietly present
  // another segment's history as this round's, so scoping is dropped and the reason
  // said out loud.
  const scopeUnplaceableReason =
    scopeRound !== null && scopeRound.placement.kind === 'unplaceable'
      ? scopeRound.placement.reason
      : null
  const scopeSegIdx = scopeUnplaceableReason !== null ? null : openRoundPos

  const forced = useMemo(() => {
    const s = new Set<number>((opts.forcedIdxs ?? []).filter((i) => i >= 0))
    s.add(a)
    if (mode === 'range' && b !== undefined) s.add(b)
    return s
  }, [opts.forcedIdxs, a, b, mode])

  // Scoping only narrows anything when the open round does not own every commit.
  // A single-round issue's Initial QC owns all of them, so this is a no-op there and
  // those issues behave exactly as they did before rounds existed.
  const scoped =
    !showAll &&
    scopeSegIdx !== null &&
    orderedCommits.some((c) => c.segIdx !== scopeSegIdx)

  // Interior Gap segments — the "draft gap" a user creates by editing after an
  // approval and before opening the next round. Only gaps *between* two rounds are
  // collapsible: a trailing gap is the newest work on the track and hiding it behind
  // a marker would be surprising rather than helpful.
  const gapRuns = useMemo(() => {
    if (!showAll) return []
    return segments
      .map((segment, segIdx) => ({ segment, segIdx }))
      .filter(
        ({ segment, segIdx }) =>
          segment.kind === 'gap' &&
          segIdx < segments.length - 1 &&
          segment.commits.length > 0,
      )
      .map(({ segIdx }) => orderedCommits.flatMap((c, i) => (c.segIdx === segIdx ? [i] : [])))
      .filter((run) => run.length > 0)
  }, [showAll, segments, orderedCommits])

  const [expandedGaps, setExpandedGaps] = useState<Set<string>>(new Set())
  const gapKey = (run: number[]) => `gap-${run[0]}-${run[run.length - 1]}`
  const toggleGap = (key: string) =>
    setExpandedGaps((prev) => {
      const next = new Set(prev)
      if (next.has(key)) next.delete(key)
      else next.add(key)
      return next
    })

  // Indices hidden behind a collapsed gap marker.
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
        .filter(({ file_changed, statuses, origIdx, segIdx }) => {
          if (forced.has(origIdx)) return true
          if (collapsedIdxs.has(origIdx)) return false
          if (showAll) return true
          if (scopeSegIdx !== null && segIdx !== scopeSegIdx) return false
          return file_changed || statuses.length > 0
        }),
    [orderedCommits, forced, collapsedIdxs, showAll, scopeSegIdx],
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

  /**
   * U1/W5: divergence is a Gap property. A Gap at position `k` whose `continuity`
   * is not `linear` says its two bounds are not ancestrally connected, and the
   * older of those bounds is the previous round's closing commit — which lives in
   * the segment at `k - 1`. So the discontinuity falls between `segIdx <= k - 1`
   * and `segIdx >= k`, and that is where the break is drawn.
   */
  const brokenGapSegIdxs = useMemo(
    () =>
      segments.flatMap((segment, segIdx) =>
        segment.kind === 'gap' && segment.continuity.kind !== 'linear'
          ? [
              {
                segIdx,
                continuity: segment.continuity.kind,
                mergeBase:
                  segment.continuity.kind === 'diverged' ? segment.continuity.merge_base : null,
              },
            ]
          : [],
      ),
    [segments],
  )

  const breaks = useMemo<TrackBreak[]>(
    () =>
      brokenGapSegIdxs.flatMap((g) => {
        const beforeSlot = visibleCommits.findIndex((c) => c.segIdx >= g.segIdx)
        // Nothing on one side of the break is on the track, so there is no boundary
        // between visible commits to mark.
        if (beforeSlot <= 0) return []
        return [{ ...g, beforeSlot }]
      }),
    [brokenGapSegIdxs, visibleCommits],
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
    // The comparison baseline for a single-handle picker is the open round's anchor
    // (what the round's work is measured against), falling back to the oldest commit
    // on the track.
    const baseIdx =
      scopeRound !== null
        ? (visibleCommits.find((c) => c.hash === scopeRound.opened_at)?.origIdx ??
          visibleCommits[0]?.origIdx ??
          0)
        : (visibleCommits[0]?.origIdx ?? 0)
    fromCommit =
      visibleCommits.find((c) => c.origIdx === baseIdx) ??
      visibleCommits.filter((c) => c.origIdx <= (toCommit?.origIdx ?? 0))[0] ??
      visibleCommits[0]
  }

  const detached =
    fromCommit !== undefined &&
    toCommit !== undefined &&
    brokenGapSegIdxs.some((g) => fromCommit!.segIdx < g.segIdx && toCommit!.segIdx >= g.segIdx)

  const spannedCount =
    fromCommit && toCommit ? Math.max(0, toCommit.origIdx - fromCommit.origIdx) : 0

  return {
    visibleCommits,
    gapMarkers,
    toggleGap,
    breaks,
    scopeRound,
    scoped,
    scopeUnplaceableReason,
    snapA,
    snapB,
    fromCommit,
    toCommit,
    selectedCommit: mode === 'single' ? toCommit : undefined,
    detached,
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
  const { visibleCommits, gapMarkers, toggleGap, breaks, viewportRef } = picker
  const n = visibleCommits.length

  return (
    <>
      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
        <div style={{ display: 'flex', alignItems: 'baseline', gap: 8, minWidth: 0 }}>
          <Text size="sm" fw={700}>{title}</Text>
          {picker.scoped && picker.scopeRound && (
            <Text size="xs" c="dimmed" data-testid="picker-scope">
              scoped to {picker.scopeRound.name}
            </Text>
          )}
        </div>
        <Checkbox
          label="Show all commits"
          checked={showAll}
          onChange={(e) => onShowAllChange(e.currentTarget.checked)}
        />
      </div>

      {/*
        The scope the track could not honour, named where the track is (D4: the reason
        is surfaced, never an error). Full history is showing instead — which is what
        the absent "scoped to …" note above already implies, said explicitly because a
        picker that silently widens is indistinguishable from one that misleads.
      */}
      {picker.scopeUnplaceableReason && picker.scopeRound && (
        <Text size="xs" c="orange" ta="center" data-testid="picker-scope-unplaceable">
          {picker.scopeRound.name}'s commits could not be placed —{' '}
          {unplaceableReasonText(picker.scopeUnplaceableReason)}. Showing the full history
          instead.
        </Text>
      )}

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
            {/*
              U1: the history is not one path here, so the track says so where the
              join actually fails rather than letting the slider imply continuity.
            */}
            {breaks.map((brk) => (
              <div
                key={brk.segIdx}
                data-testid={`picker-break-${brk.segIdx}`}
                data-continuity={brk.continuity}
                title={
                  brk.continuity === 'diverged'
                    ? `History diverges here — the two sides meet at ${brk.mergeBase?.slice(0, 7)}, so they are not one path`
                    : 'These commits share no history — no diff across this point is meaningful'
                }
                style={{
                  position: 'absolute',
                  top: 0,
                  bottom: 0,
                  left: slotBoundaryLeft(brk.beforeSlot, n),
                  width: 0,
                  borderLeft: '2px dashed var(--mantine-color-orange-6)',
                  zIndex: 4,
                }}
              />
            ))}
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
 *
 * U1: when the two ends are separated by a non-linear Gap the arrow is broken and
 * the count is withheld — "N commits" across histories that do not connect would be
 * a number about nothing.
 */
export function ComparisonReceipt({ picker }: { picker: RoundPicker }) {
  const { fromCommit, toCommit, spannedCount, detached } = picker
  if (!toCommit) return null
  const from = fromCommit?.hash.slice(0, 7) ?? '—'
  const to = toCommit.hash.slice(0, 7)
  return (
    <Text
      size="xs"
      c={detached ? 'orange' : 'dimmed'}
      ta="center"
      data-testid="comparison-receipt"
      data-detached={detached ? 'true' : undefined}
      style={{ fontVariantNumeric: 'tabular-nums' }}
    >
      <span style={{ fontFamily: 'monospace' }}>{from}</span>
      {detached ? ' ⇢ ' : ' → '}
      <span style={{ fontFamily: 'monospace' }}>{to}</span>
      {' · '}
      {detached ? 'histories not connected' : `${spannedCount} commit${spannedCount === 1 ? '' : 's'}`}
    </Text>
  )
}
