// Segment-aware commit picker, shared by the Notify (two handles), Review and
// Approve (one handle) panels of IssueDetailModal.
//
// U1: the track is rendered segment by segment from `segments`. Nothing in here
// computes round windows, coverage or gap runs — the API already owns which commit
// belongs to which segment (D7), so scoping is a read of `segIdx`, the rail's entries
// *are* the segments, and a Gap whose bounds are not ancestrally connected puts a
// visible break in the track — and a broken connector on the rail — instead of letting
// a continuous slider imply a linear path that does not exist.
//
// `useRoundPicker` owns everything derived from that — the visible commit window,
// the segment rail, the breaks, handle snapping and the resolved from/to selection —
// and `RoundCommitPickerTrack` renders it. The panels keep owning the raw state
// (which index each handle is on, whether the density checkbox is checked) so that
// their existing reset-on-issue-change effects keep working unchanged.

import { useEffect, useMemo, useRef, useState, type ReactNode } from 'react'
import {
  Button,
  Checkbox,
  Popover,
  ScrollArea,
  Slider,
  Text,
  UnstyledButton,
} from '@mantine/core'
import type {
  GapContinuity,
  GapSegment,
  RoundSegment,
  Segment,
  UnplaceableReason,
} from '~/api/rounds'
import { CommitSlider } from '~/components/CommitSlider'
import {
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

/**
 * One entry in the segment rail: the thread's structure, rendered *off* the track.
 *
 * Reach used to live on the track itself, as chips pinned to the boundary before the
 * first visible slot. Every out-of-scope segment is older than the scope, so that
 * boundary was always slot 0 — the chips were always in the same place, carried no
 * positional meaning, and piled into the left corner once a third round existed. Worse,
 * an unplaceable scope round emptied the scope, which erased every chip exactly when
 * the structure most needed explaining.
 *
 * The rail separates two claims the track conflated:
 *
 * - **Order** is round order, which the model always knows — `segments` is strictly
 *   alternating and oldest-first, derived from comment order in the issue, not from git
 *   ancestry. So a rail entry's position is honest even for rounds on unrelated branches.
 * - **Connection** is a claim about commits, and belongs to the gaps alone via
 *   `continuity`. A `diverged` or `unrelated` gap renders as a broken or severed
 *   connector, so the rail never draws a chain it cannot back up.
 */
interface RailSegment {
  /** Position in `segments`. Identity is position, never hash or index (U7). */
  pos: number
  kind: 'round' | 'gap'
  /** `"Round 2"` for a round; gaps are unnamed and positional. */
  name: string | null
  /** Commits owned. Zero for an `Unplaceable` segment (I5) or a genuinely empty gap. */
  count: number
  /** Non-null when this segment could not be placed; it then owns no commits. */
  unplaceable: UnplaceableReason | null
  /** Gaps only: how the gap's two bounds relate. Null on rounds. */
  continuity: GapContinuity['kind'] | null
  /** Set only for a `diverged` gap: the commit its two histories share. */
  mergeBase: string | null
  /** In the default scope, so on the track without the user asking. */
  inScope: boolean
  /** Currently contributing commits to the track. */
  onTrack: boolean
  /** Whether clicking does anything: owns commits and is not permanently on. */
  selectable: boolean
  /** Rounds only: whether the round is still open. Null on gaps. */
  state: 'open' | 'closed' | null
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
  /** Every segment, oldest-first, for the rail. Always populated. */
  railSegments: RailSegment[]
  /** Put a segment's commits on the track, or take them off. Keyed by position. */
  toggleSegment: (pos: number) => void
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
  /** U6/U7: whether the `to` handle may occupy this visible slot. */
  endAllowed: (visibleIdx: number) => boolean
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
  /**
   * D16 (density): show every commit on the track, not only the file-changing or
   * comment-named ones. **Never widens the reach** — which segments are on the track is
   * the rail's job, and the two axes never touch.
   */
  showAll: boolean
  /**
   * U6/U7: how far the *`to`* end may reach. The `from` end is always free.
   *
   * - `'any'` — Review: a review records what the reviewer read, and reading an older
   *   round's commit is legitimate.
   * - `'scope-and-newer'` — Notify: `to` means "the newest state I claim to have
   *   addressed", so it may not sit in a round that already closed. Keyed on position,
   *   not segment kind, so trailing-gap drift (newer than the scope) qualifies.
   * - `'scope-round-only'` — Approve: approval closes a round **at** a commit, so the
   *   commit must belong to that round. Excludes the trailing gap too: approving drift
   *   no round covers would bypass the model (S5 makes *start a new round* the
   *   resolution for `changes_after_approval`).
   *
   * Defaults to `'any'`. The CLI stays permissive by design (U8).
   */
  endReach?: 'any' | 'scope-and-newer' | 'scope-round-only'
}

export function useRoundPicker(opts: UseRoundPickerOptions): RoundPicker {
  const { orderedCommits, segments, mode, a, setA, b, setB, showAll } = opts
  const endReach = opts.endReach ?? 'any'

  // Scope is the open round's own commit ownership — no window arithmetic. The
  // round's `commits` already include its `opened_at` (W2), which is the commit its
  // work is measured against, so the anchor stays on the track exactly as before.
  // U5: scope is the newest Round **plus the trailing Gap when one is last**, not
  // `activeRoundPos`. That accessor is null unless the last segment is a Round, so the
  // moment a round was approved — I3 appends a trailing Gap — the scope filter stopped
  // firing and the whole history appeared. Keeping the closed round in scope means
  // approving does not change what is on screen, and drift landing in the trailing gap
  // shows up without touching a control.
  const scopeRoundPos = useMemo(() => {
    for (let i = segments.length - 1; i >= 0; i--) if (segments[i].kind === 'round') return i
    return null
  }, [segments])
  const scopeRound = useMemo(
    () => (scopeRoundPos === null ? null : (segments[scopeRoundPos] as RoundSegment)),
    [segments, scopeRoundPos],
  )
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
  // The scoped segments: the newest Round, plus the trailing Gap when the last segment
  // is one. Null-ish (empty) when the round is unplaceable, which drops scoping.
  const scopePositions = useMemo(() => {
    const s = new Set<number>()
    if (scopeUnplaceableReason !== null || scopeRoundPos === null) return s
    s.add(scopeRoundPos)
    const last = segments.length - 1
    if (segments[last]?.kind === 'gap') s.add(last)
    return s
  }, [scopeUnplaceableReason, scopeRoundPos, segments])
  const hasScope = scopePositions.size > 0

  const forced = useMemo(() => {
    const s = new Set<number>((opts.forcedIdxs ?? []).filter((i) => i >= 0))
    s.add(a)
    if (mode === 'range' && b !== undefined) s.add(b)
    return s
  }, [opts.forcedIdxs, a, b, mode])

  // Scoping only narrows anything when the scope does not already own every commit.
  // A single-round issue's Initial QC owns all of them, so this is a no-op there and
  // those issues behave exactly as they did before rounds existed.
  const scoped = hasScope && orderedCommits.some((c) => !scopePositions.has(c.segIdx))

  // The segments on the track by default. Normally the scope; when the scope round is
  // unplaceable (D4/S4) the scope is empty, and rather than a track of stray forced
  // defaults everything that owns commits goes on — the full-history fallback, which
  // the rail is now there to explain instead of leaving unexplained.
  const defaultTrackPositions = useMemo(() => {
    if (hasScope) return scopePositions
    const s = new Set<number>()
    segments.forEach((seg, i) => {
      if (seg.commits.length > 0) s.add(i)
    })
    return s
  }, [hasScope, scopePositions, segments])

  // Keyed by segment position: stable across re-renders and across a refetch that adds
  // commits, unlike a first/last-commit-index key.
  const [addedSegs, setAddedSegs] = useState<Set<number>>(new Set())
  const toggleSegment = (pos: number) =>
    setAddedSegs((prev) => {
      const next = new Set(prev)
      if (next.has(pos)) next.delete(pos)
      else next.add(pos)
      return next
    })

  const onTrackPositions = useMemo(() => {
    const s = new Set(defaultTrackPositions)
    for (const pos of addedSegs) s.add(pos)
    return s
  }, [defaultTrackPositions, addedSegs])

  const railSegments = useMemo<RailSegment[]>(
    () =>
      segments.map((segment, pos) => {
        const gap = segment.kind === 'gap' ? (segment as GapSegment) : null
        return {
          pos,
          kind: segment.kind,
          name: segment.kind === 'round' ? (segment as RoundSegment).name : null,
          count: segment.commits.length,
          unplaceable:
            segment.placement.kind === 'unplaceable' ? segment.placement.reason : null,
          continuity: gap ? gap.continuity.kind : null,
          mergeBase:
            gap && gap.continuity.kind === 'diverged' ? gap.continuity.merge_base : null,
          inScope: defaultTrackPositions.has(pos),
          onTrack: onTrackPositions.has(pos),
          // An in-scope segment cannot be taken off the track: the round being worked in
          // is the whole point of the picker. Everything else toggles, including a
          // segment across a severed gap — the UI never blocks reach, the receipt just
          // declines to claim a diff (U8: the CLI is permissive too).
          selectable: segment.commits.length > 0 && !defaultTrackPositions.has(pos),
          state: segment.kind === 'round' ? (segment as RoundSegment).state : null,
        }
      }),
    [segments, defaultTrackPositions, onTrackPositions],
  )

  const visibleCommits = useMemo(
    () =>
      orderedCommits
        .map((c, i) => ({ ...c, origIdx: i }))
        .filter(({ file_changed, statuses, origIdx, segIdx }) => {
          // Defaults and pins are always offered — the notify base may legitimately be
          // the previous round's approval, which lives outside the scope.
          if (forced.has(origIdx)) return true
          // Two orthogonal axes, and only two: the rail decides *which segments* are on
          // the track, the density checkbox decides *how much of them* is shown. Neither
          // reaches into the other's job, which is the whole correction here.
          if (!onTrackPositions.has(segIdx)) return false
          return showAll || file_changed || statuses.length > 0
        }),
    [orderedCommits, forced, showAll, onTrackPositions],
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

  // U6/U7: which slots the `to` handle may land on. `from` is unconstrained.
  const endAllowed = (c: PickerCommit): boolean => {
    if (endReach === 'any' || !hasScope) return true
    if (endReach === 'scope-round-only') return c.segIdx === scopeRoundPos
    // 'scope-and-newer': in scope, or in a segment newer than the scope round.
    return scopePositions.has(c.segIdx) || c.segIdx > (scopeRoundPos ?? -1)
  }

  /** Nearest slot at or older than `visibleIdx` that the `to` handle may occupy. */
  const snapToAllowedEnd = (visibleIdx: number): number => {
    if (visibleCommits.length === 0) return 0
    if (visibleCommits[visibleIdx] !== undefined && endAllowed(visibleCommits[visibleIdx]))
      return visibleIdx
    // Prefer the newest allowed slot that is not newer than where the handle was put,
    // so a constrained handle never silently jumps *forward* past commits the user can
    // see. Falling back to the newest allowed slot at all keeps the handle on the track.
    for (let i = Math.min(visibleIdx, visibleCommits.length - 1); i >= 0; i--) {
      if (endAllowed(visibleCommits[i])) return i
    }
    for (let i = visibleIdx + 1; i < visibleCommits.length; i++) {
      if (endAllowed(visibleCommits[i])) return i
    }
    return visibleIdx
  }

  const snapA = mode === 'single' ? snapToAllowedEnd(snapToVisible(a)) : snapToVisible(a)
  const snapB =
    mode === 'range' && b !== undefined ? snapToAllowedEnd(snapToVisible(b)) : snapA

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
    railSegments,
    toggleSegment,
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
    endAllowed: (visibleIdx: number) => {
      const c = visibleCommits[visibleIdx]
      return c === undefined ? true : endAllowed(c)
    },
    onA: (visibleIdx) =>
      setA(
        visibleCommits[mode === 'single' ? snapToAllowedEnd(visibleIdx) : visibleIdx]?.origIdx ?? a,
      ),
    onB: (visibleIdx) => setB?.(visibleCommits[snapToAllowedEnd(visibleIdx)]?.origIdx ?? b ?? a),
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
/**
 * The history menu: the thread's structure as a **vertical** timeline behind a
 * `History ▾` button.
 *
 * This is the third design for reach, and the first that is not a small graphic the
 * reader has to decode. The two that failed both tried to fit the structure into the
 * track's own one-dimensional space:
 *
 * 1. Chips pinned to a track boundary. Every out-of-scope segment is older than the
 *    scope, so the boundary was always slot 0 — the chips never moved, so their position
 *    meant nothing, and a third round piled them into the corner.
 * 2. A horizontal rail of chips joined by drawn connectors. At that size a connector is
 *    a few pixels of dashes: the continuity distinction it existed to carry was
 *    illegible, and pills reading `Round 2 ·2` next to `Initial QC ·5` look like a tab
 *    bar, so the implied action was *switch to that round* when the actual action is
 *    *add it alongside*.
 *
 * Both failures are the same failure: 1D leaves room for glyphs but not for words.
 * Vertically there is room to simply say `5 commits`, `no shared history`, `its branch
 * is unavailable locally` — and a spine drawn as a border on each row's gutter joins up
 * by construction, with no position arithmetic to get wrong.
 *
 * Newest first, matching `git log` rather than the track's left-to-right age order —
 * the segment you are working in is the one you want under the cursor.
 */
function HistoryMenu({
  railSegments,
  onToggle,
  scopeFellBack,
}: {
  railSegments: RailSegment[]
  onToggle: (pos: number) => void
  /**
   * True when there is no scope to speak of because the newest round is `Unplaceable`
   * (D4/S4) and the track fell back to the full history. The rows must not then describe
   * an older round as the one being worked in — it is on the track by default, but only
   * because nothing better could be resolved.
   */
  scopeFellBack: boolean
}) {
  // Declared before the early return below so the hook order never changes with the data.
  const [opened, setOpened] = useState(false)

  // A single-round thread has no structure to navigate; showing it a menu naming the one
  // round it has would be noise. Those threads behave as they did before rounds existed.
  const worthShowing =
    railSegments.filter((seg) => seg.kind === 'round').length > 1 ||
    railSegments.some(
      (seg) =>
        seg.kind === 'gap' &&
        (seg.count > 0 || seg.continuity !== 'linear' || seg.unplaceable !== null),
    )
  if (!worthShowing) return null

  const added = railSegments.filter((seg) => seg.onTrack && !seg.inScope).length
  // Newest-first for display; `pos` still indexes the oldest-first model.
  const rows = [...railSegments].reverse()

  return (
    <Popover
      width={344}
      position="bottom-start"
      shadow="md"
      withinPortal
      // Accessible menu behaviour: focus moves into the dropdown on open.
      //
      // Known rough edge, unchanged from before this menu existed: Escape closes the
      // whole issue modal rather than just this dropdown. Mantine's Modal implements
      // `closeOnEscape` with a window listener that a capture-phase listener here does
      // not preempt, so scoping it properly means plumbing `closeOnEscape` through
      // IssueDetailModal. Dismiss the menu by clicking `History` again.
      trapFocus
      opened={opened}
      onChange={setOpened}
    >
      <Popover.Target>
        <Button
          variant="default"
          size="compact-xs"
          data-testid="history-menu-trigger"
          onClick={() => setOpened((o) => !o)}
          style={{ alignSelf: 'flex-start', fontWeight: 500 }}
        >
          History{added > 0 ? ` · +${added}` : ''} ▾
        </Button>
      </Popover.Target>
      <Popover.Dropdown data-testid="history-menu" p="xs">
        <Text size="xs" c="dimmed" mb={8}>
          Newest first · tick to add to the track
        </Text>
        {rows.map((seg, i) =>
          seg.kind === 'round' ? (
            <HistoryRoundRow
              key={seg.pos}
              seg={seg}
              onToggle={onToggle}
              scopeFellBack={scopeFellBack}
              isFirst={i === 0}
              isLast={i === rows.length - 1}
            />
          ) : (
            <HistoryGapRow key={seg.pos} seg={seg} onToggle={onToggle} isFirst={i === 0} />
          ),
        )}
      </Popover.Dropdown>
    </Popover>
  )
}

const SPINE_X = 8
const DOT_TOP = 6

/**
 * One row of the timeline: a fixed-width gutter carrying the spine, and content beside it.
 *
 * The spine is a `border-left` on the gutter of *every* row, so consecutive rows' spines
 * meet exactly where the rows meet — there is nothing to align and nothing to compute.
 * That is the whole reason the vertical form works where the horizontal one did not.
 */
function SpineRow({
  children,
  continuity,
  dot,
  spineFrom,
  spineTo,
  rowProps,
}: {
  children: ReactNode
  /** Which line style the spine takes *through this row*. */
  continuity: 'solid' | 'dashed' | 'severed'
  dot: ReactNode
  /** Start the spine at the dot instead of the row's top edge (newest row). */
  spineFrom?: 'dot'
  /** Stop the spine at the dot instead of the row's bottom edge (oldest row). */
  spineTo?: 'dot'
  rowProps?: Record<string, string | undefined>
}) {
  return (
    <div style={{ display: 'flex', alignItems: 'stretch', minHeight: 22 }} {...rowProps}>
      <div style={{ position: 'relative', flex: `0 0 ${SPINE_X + 12}px` }}>
        {continuity !== 'severed' && (
          <div
            style={{
              position: 'absolute',
              left: SPINE_X,
              top: spineFrom === 'dot' ? DOT_TOP + 5 : 0,
              bottom: spineTo === 'dot' ? undefined : 0,
              height: spineTo === 'dot' ? DOT_TOP + 5 : undefined,
              borderLeft: `2px ${continuity} ${
                continuity === 'dashed'
                  ? 'var(--mantine-color-orange-5)'
                  : 'var(--mantine-color-gray-4)'
              }`,
            }}
          />
        )}
        {/*
          A severed gap gets no spine at all, but a bar across where the spine would be:
          "these two rounds are on branches that meet nowhere" is a different fact from
          "they diverged and meet upstream", and a dashed line reads as the second.
        */}
        {continuity === 'severed' && (
          <div
            style={{
              position: 'absolute',
              left: SPINE_X - 4,
              top: 8,
              width: 11,
              borderTop: '2px solid var(--mantine-color-orange-6)',
            }}
          />
        )}
        {dot}
      </div>
      <div style={{ flex: 1, minWidth: 0, paddingBottom: 8 }}>{children}</div>
    </div>
  )
}

function HistoryRoundRow({
  seg,
  onToggle,
  scopeFellBack,
  isFirst,
  isLast,
}: {
  seg: RailSegment
  onToggle: (pos: number) => void
  scopeFellBack: boolean
  isFirst: boolean
  isLast: boolean
}) {
  const dotColor = seg.unplaceable
    ? 'var(--mantine-color-orange-5)'
    : seg.onTrack
      ? 'var(--mantine-color-blue-6)'
      : 'var(--mantine-color-gray-5)'

  const dot = (
    <div
      style={{
        position: 'absolute',
        left: SPINE_X - 3,
        top: DOT_TOP,
        width: 10,
        height: 10,
        borderRadius: '50%',
        backgroundColor: seg.onTrack ? dotColor : 'var(--mantine-color-body)',
        border: `2px solid ${dotColor}`,
        boxSizing: 'border-box',
        zIndex: 1,
      }}
    />
  )

  // Said in words, because there is room for words here. `·5` was the old rail's whole
  // problem in miniature: correct, compact, and unreadable.
  const detail = seg.unplaceable
    ? unplaceableReasonText(seg.unplaceable)
    : seg.count === 0
      ? 'no commits'
      : `${seg.count} commit${seg.count === 1 ? '' : 's'}`

  const suffix = seg.unplaceable
    ? 'could not be placed'
    : // In the fallback there is no round being worked in, so say only what is true of
      // the round itself. Calling an older round "the one you just closed" because it
      // happened to be the newest *placeable* segment would be a plain misstatement.
      seg.inScope && !scopeFellBack
      ? seg.state === 'open'
        ? 'current round'
        : 'just closed'
      : seg.state === 'closed'
        ? 'closed'
        : 'open'

  const content = (
    <div style={{ display: 'flex', justifyContent: 'space-between', gap: 8 }}>
      <div style={{ minWidth: 0 }}>
        <Text size="xs" fw={600} data-testid={`history-name-${seg.pos}`}>
          {seg.name}
        </Text>
        <Text size="xs" c={seg.unplaceable ? 'orange' : 'dimmed'}>
          {suffix ? `${suffix} · ${detail}` : detail}
        </Text>
      </div>
      {seg.inScope ? (
        <Text size="xs" c="dimmed" style={{ whiteSpace: 'nowrap' }}>
          always shown
        </Text>
      ) : seg.selectable ? (
        <Checkbox
          size="xs"
          checked={seg.onTrack}
          readOnly
          tabIndex={-1}
          styles={{ input: { pointerEvents: 'none' } }}
        />
      ) : null}
    </div>
  )

  const rowProps = {
    'data-testid': `rail-round-${seg.pos}`,
    'data-unplaceable': seg.unplaceable ?? undefined,
    ...railTestProps(seg),
  }

  const row = (
    <SpineRow
      continuity="solid"
      dot={dot}
      spineFrom={isFirst ? 'dot' : undefined}
      spineTo={isLast ? 'dot' : undefined}
      rowProps={rowProps}
    >
      {content}
    </SpineRow>
  )

  return seg.selectable ? (
    <UnstyledButton
      onClick={() => onToggle(seg.pos)}
      style={{ display: 'block', width: '100%' }}
      aria-label={`${seg.onTrack ? 'Remove' : 'Add'} ${seg.name}'s commits`}
    >
      {row}
    </UnstyledButton>
  ) : (
    row
  )
}

function railTestProps(seg: RailSegment) {
  return {
    'data-on-track': seg.onTrack ? 'true' : 'false',
    'data-in-scope': seg.inScope ? 'true' : 'false',
    'data-selectable': seg.selectable ? 'true' : 'false',
  }
}

/**
 * A gap row. The gap is the join between two rounds, so it is drawn *as the spine* —
 * its continuity is the spine's line style — rather than as a station of its own.
 */
function HistoryGapRow({
  seg,
  onToggle,
  isFirst,
}: {
  seg: RailSegment
  onToggle: (pos: number) => void
  isFirst: boolean
}) {
  const style =
    seg.continuity === 'unrelated' ? 'severed' : seg.continuity === 'diverged' ? 'dashed' : 'solid'

  const note =
    // An unplaceable gap's commits are unknown, not absent, so a plain spine across it
    // would assert a continuity nothing established.
    seg.unplaceable
      ? 'commits between these rounds could not be listed'
      : seg.continuity === 'unrelated'
      ? 'no shared history — no diff across this point is meaningful'
      : seg.continuity === 'diverged'
        ? `histories diverge — they meet at ${seg.mergeBase?.slice(0, 7) ?? 'an unknown commit'}`
        : null

  const countText =
    seg.count > 0
      ? `${seg.count} commit${seg.count === 1 ? '' : 's'} between rounds`
      : null

  const rowProps = {
    'data-testid': `rail-gap-${seg.pos}`,
    'data-continuity': seg.continuity ?? undefined,
    ...railTestProps(seg),
  }

  // A linear, empty gap has nothing to say. It still draws its length of spine, so the
  // two rounds it joins read as consecutive rather than as one row butted against another.
  if (note === null && countText === null) {
    return (
      <SpineRow continuity="solid" dot={null} spineFrom={isFirst ? 'dot' : undefined} rowProps={rowProps}>
        <div style={{ height: 6 }} />
      </SpineRow>
    )
  }

  const content = (
    <div style={{ display: 'flex', justifyContent: 'space-between', gap: 8 }}>
      <div style={{ minWidth: 0 }}>
        {countText && (
          <Text size="xs" c="dimmed">
            {countText}
          </Text>
        )}
        {note && (
          <Text size="xs" c="orange" data-testid={`history-continuity-${seg.pos}`}>
            {note}
          </Text>
        )}
      </div>
      {seg.selectable ? (
        <Checkbox
          size="xs"
          checked={seg.onTrack}
          readOnly
          tabIndex={-1}
          styles={{ input: { pointerEvents: 'none' } }}
        />
      ) : null}
    </div>
  )

  const row = (
    <SpineRow
      continuity={style}
      dot={null}
      spineFrom={isFirst ? 'dot' : undefined}
      rowProps={rowProps}
    >
      {content}
    </SpineRow>
  )

  return seg.selectable ? (
    <UnstyledButton
      onClick={() => onToggle(seg.pos)}
      style={{ display: 'block', width: '100%' }}
      aria-label={`${seg.onTrack ? 'Remove' : 'Add'} the commits between rounds`}
    >
      {row}
    </UnstyledButton>
  ) : (
    row
  )
}

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
  const { visibleCommits, railSegments, toggleSegment, breaks, viewportRef } = picker
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
        {/*
          D16: density only. This used to be "Show all commits" and also left the round
          scope and enabled the segment markers — three axes on one control, which is why
          seeing one extra commit in the current round meant showing the entire history.
          Reach now lives on the rail; this governs only how much of whatever is on the
          track gets shown, which is why it says "on the track" rather than "in this
          round" — the rail can put more than the round there.
        */}
        <Checkbox
          label="Show every commit on the track"
          checked={showAll}
          data-testid="show-all-in-round"
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

      <HistoryMenu
        railSegments={railSegments}
        onToggle={toggleSegment}
        scopeFellBack={picker.scopeUnplaceableReason !== null}
      />

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
            // Symmetric: this was 16/40, which parked the whole track 24px left of
            // centre inside the panel. `offsetScrollbars` on the ScrollArea already
            // reserves the scrollbar gutter, so the extra right padding bought nothing.
            paddingLeft: 40,
            paddingRight: 40,
          }}
          data-testid={testId}
        >
          <div style={{ position: 'relative', height: 8 }}>
            {visibleCommits.map((c, i) => {
              const pct = n > 1 ? i / (n - 1) : 0.5
              const left = `calc(10px + ${pct * 100}% - ${pct * 20}px)`
              return (
                <div
                  key={i}
                  // U6: a slot the `to` handle may not occupy is dimmed, so the
                  // constraint is visible before the user drags into it rather than
                  // being felt as a handle that mysteriously refuses to land.
                  data-end-allowed={picker.endAllowed(i) ? 'true' : 'false'}
                  style={{
                    position: 'absolute',
                    left,
                    transform: 'translateX(-50%)',
                    display: 'flex',
                    gap: 2,
                    opacity: picker.endAllowed(i) ? 1 : 0.35,
                  }}
                >
                  {STATUS_ORDER.filter((s) => c.statuses.includes(s)).map((s) => (
                    <span
                      key={s}
                      title={s}
                      data-testid={`commit-dot-${c.hash.slice(0, 7)}-${s}`}
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
