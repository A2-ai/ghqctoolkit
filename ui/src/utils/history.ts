import type { Gap, IssueCommit, IssueStatusResponse, RoundInfo, SegmentRef } from '~/api/issues'
import { latestRound, roundByIndex } from '~/api/issues'

/**
 * One row of the History dropdown, resolved against `rounds`/`drift` (§22 / M2).
 *
 * The **order** comes from `status.history` and nowhere else: W6's two positional
 * suppression rules (skip round 1's preceding gap; emit `drift` only once the latest
 * round is closed) are the server's (D30/U7), so this module reads the projection and
 * never rebuilds it.
 */
export interface HistorySegment {
  ref: SegmentRef
  /** Stable identity for selection state. Kind is part of it: a round and its preceding
   *  gap share a `round_index`. */
  key: string
  label: string
  /** Newest-first, exactly as the wire carries them. */
  commits: IssueCommit[]
  /** D22/D31: this segment's history does not continue the previous segment's. A round
   *  is never itself divergent — its preceding gap is. */
  divergent: boolean
  round: RoundInfo
  /** D81: the tail block — the latest round and its drift, the region a Notify `to`
   *  handle may rest in (D79). Pinned in the dropdown (D80). */
  isTail: boolean
}

/** How many of these commits changed the QC'd file. */
export function countFileChanging(commits: IssueCommit[]): number {
  return commits.filter((commit) => commit.file_changed).length
}

/**
 * "5 commits (3 file changing)" — the two raw facts about a set of commits, rather than
 * one derived number.
 *
 * An earlier version showed only the commits the slider draws by default, which made a
 * gap holding one irrelevant commit read as `0` and left no way to tell "nothing here"
 * from "nothing *interesting* here". Showing both means a row can be honest about owning
 * commits the slider will not draw until "Show all commits" is on.
 *
 * Note the parenthetical is **not** the count of visible ticks: the slider also shows
 * commits that carry a QC status without touching the file. It is the file-changing count,
 * which is the question a reviewer actually asks of a span.
 */
export function describeCommits(commits: IssueCommit[]): string {
  const total = commits.length
  const label = `${total} commit${total === 1 ? '' : 's'}`
  if (total === 0) return label
  return `${label} (${countFileChanging(commits)} file changing)`
}

export function segmentKey(ref: SegmentRef): string {
  return `${ref.kind}:${ref.round_index}`
}

function segmentLabel(ref: SegmentRef, round: RoundInfo): string {
  switch (ref.kind) {
    case 'round':
      return `Round ${round.index}`
    // A gap belongs to the round it *precedes* (D9), so it is named by where it leads.
    case 'gap':
      return `Before round ${round.index}`
    case 'drift':
      return `Since round ${round.index}'s approval`
  }
}

function segmentGap(status: IssueStatusResponse, ref: SegmentRef, round: RoundInfo): Gap | null {
  if (ref.kind === 'gap') return round.preceding_gap
  if (ref.kind === 'drift') return status.drift
  return null
}

/** Resolves `status.history` into rows, in the server's order. */
export function buildHistory(status: IssueStatusResponse): HistorySegment[] {
  const latestIndex = latestRound(status).index
  return status.history.map((ref) => {
    const round = roundByIndex(status, ref.round_index)
    const gap = segmentGap(status, ref, round)
    return {
      ref,
      key: segmentKey(ref),
      label: segmentLabel(ref, round),
      commits: gap ? gap.commits : round.commits,
      divergent: gap ? gap.divergent : false,
      round,
      // D81: latest round ∪ drift, one contiguous region — both are "now".
      isTail: ref.kind === 'drift' || (ref.kind === 'round' && ref.round_index === latestIndex),
    }
  })
}

/** D71: the default selection — the latest round only, byte-identical to the pre-§22 view. */
export function defaultSelection(history: HistorySegment[]): string[] {
  return history.filter((segment) => segment.isTail).map((segment) => segment.key)
}

/** A commit in the flattened, oldest-first slider order. */
export interface FlatCommit extends IssueCommit {
  /** The segment it came from, so the slider can label and group. */
  segmentKey: string
  segmentLabel: string
  /**
   * D74: this commit is **not** adjacent in history to the one before it. One marker,
   * two causes — a divergent segment boundary (D22), or a segment the selection skipped.
   */
  breakBefore: boolean
}

export interface FlatHistory {
  commits: FlatCommit[]
  /**
   * D79: the first index a Notify `to` handle may rest at. `-1` when no tail commit is
   * displayed at all, which callers must treat as "no legal `to`".
   */
  tailStart: number
}

/**
 * Flattens the selected segments into one oldest-first commit list.
 *
 * Each segment's own commits arrive newest-first (D8/M4) and the segments themselves run
 * oldest→newest, so each is reversed and they are concatenated in order.
 */
export function flattenSelection(
  history: HistorySegment[],
  selected: ReadonlySet<string>,
): FlatHistory {
  const commits: FlatCommit[] = []
  let tailStart = -1
  let previousShownPosition = -1
  // A break is drawn on the next commit that actually appears. It has to *carry* across
  // an empty segment: a divergent gap with no commits is normal (D8's overlap case), and
  // attaching the marker only to that segment's own first commit would silently lose it.
  let pendingBreak = false

  history.forEach((segment, position) => {
    if (!selected.has(segment.key)) return

    // D74: one marker, two causes — this segment's history does not continue what
    // precedes it, or the selection omitted whatever sat between.
    const omittedBefore = previousShownPosition !== -1 && position !== previousShownPosition + 1
    pendingBreak = pendingBreak || segment.divergent || omittedBefore

    if (segment.isTail && tailStart === -1 && segment.commits.length > 0) {
      tailStart = commits.length
    }

    ;[...segment.commits].reverse().forEach((commit, offset) => {
      // The very first displayed commit has nothing to be discontinuous *with*, so it
      // never carries the marker — the view simply starts there.
      const breakBefore = commits.length > 0 && offset === 0 && pendingBreak
      commits.push({
        ...commit,
        segmentKey: segment.key,
        segmentLabel: segment.label,
        breakBefore,
      })
    })

    if (segment.commits.length > 0) pendingBreak = false
    previousShownPosition = position
  })

  return { commits, tailStart }
}

/**
 * D75: whether the range `(from, to]` crosses a discontinuity. When it does, order-derived
 * facts about the range are not facts — `fileChangedInRange` walks commits that are not
 * one history — so callers go conservative rather than reporting a walk they cannot trust.
 *
 * Content is unaffected: a blob-to-blob diff needs no ancestry (D22).
 */
export function rangeCrossesBreak(commits: FlatCommit[], fromIdx: number, toIdx: number): boolean {
  for (let i = Math.min(fromIdx, toIdx) + 1; i <= Math.max(fromIdx, toIdx); i++) {
    if (commits[i]?.breakBefore) return true
  }
  return false
}
