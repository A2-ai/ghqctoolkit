// Rendering helpers over the segment list.
//
// Every fact in here is read *positionally* off `segments` exactly as the API
// shaped it (design/segment-api-contract.md §2, §4). Nothing re-derives round
// scope — the backend owns that now (D7), and the four functions that used to do
// it here (`roundWindow`, `roundWindows`, `openRoundWindow`, `draftGapRuns`) are
// deleted by U4. What is left is ordering and positional lookup: segments arrive
// oldest-first carrying newest-first commits, while the pickers index into a
// single oldest-first list.

import type { RoundSegment, Segment, UnplaceableReason } from '~/api/rounds'

/** The commit shape the pickers work with. */
export interface OrderedCommit {
  hash: string
  message: string
  statuses: string[]
  file_changed: boolean
  /** Position in `segments` of the segment that owns this commit. */
  segIdx: number
}

/**
 * Every known commit as one oldest-first list, tagged with its owning segment.
 *
 * This is the only place the segment list is flattened. It is ordering, not
 * derivation: the API already decided which segment owns which commit, and the
 * track needs one monotone index space to put slider handles in.
 */
export function flattenSegmentCommits(segments: readonly Segment[]): OrderedCommit[] {
  const out: OrderedCommit[] = []
  const positionOf = new Map<string, number>()

  segments.forEach((segment, segIdx) => {
    // A segment's `commits` is newest-first; the track reads oldest-first.
    for (let i = segment.commits.length - 1; i >= 0; i--) {
      const c = segment.commits[i]
      const existing = positionOf.get(c.hash)
      if (existing !== undefined) {
        // D1: a round's `opened_at` may be the previous round's closing commit, so
        // one hash is legitimately owned by two adjacent segments (I4 exempts it).
        // It keeps its position but is attributed to the *newer* owner — that is the
        // segment whose scope it bounds, and it is what keeps a round's anchor on the
        // track while the picker is scoped to that round. Its `statuses` become the
        // union of both projections: the contract leaves this open (§7.7), and a
        // union is the only reading under which a status dot never disappears
        // depending on which side of the boundary is being drawn.
        const prev = out[existing]
        out[existing] = {
          ...prev,
          segIdx,
          statuses: [...new Set([...prev.statuses, ...c.statuses])],
        }
        continue
      }
      positionOf.set(c.hash, out.length)
      out.push({
        hash: c.hash,
        message: c.message,
        statuses: [...c.statuses],
        file_changed: c.file_changed,
        segIdx,
      })
    }
  })

  return out
}

/** The last segment. Total by I3: an open Round or a Gap, never a closed Round. */
export function activeSegment(segments: readonly Segment[]): Segment | null {
  return segments.length > 0 ? segments[segments.length - 1] : null
}

/**
 * The open round, or null when the file is not under review.
 *
 * A1: a round is open iff the last segment is a Round — which is what replaces the
 * deleted `open_round_index`.
 */
export function activeRound(segments: readonly Segment[]): RoundSegment | null {
  const last = activeSegment(segments)
  return last !== null && last.kind === 'round' ? last : null
}

/** Position of the open round in `segments`, or null when none is open. */
export function activeRoundPos(segments: readonly Segment[]): number | null {
  return activeRound(segments) !== null ? segments.length - 1 : null
}

/** Just the Round segments, oldest-first. Their `index` is 1-based and contiguous. */
export function roundSegments(segments: readonly Segment[]): RoundSegment[] {
  return segments.filter((s): s is RoundSegment => s.kind === 'round')
}

/**
 * The newest closed Round.
 *
 * This is the join the contract's §7.6 specifies for the `previous_branch` that
 * `GapContinuity` dropped: every Round declares a branch unconditionally (D5), so
 * the last closed Round's `branch` is the branch the previous approval was
 * reviewed on. A render-time read of data the caller already holds, not derivation.
 */
export function lastClosedRound(segments: readonly Segment[]): RoundSegment | null {
  for (let i = segments.length - 1; i >= 0; i--) {
    const s = segments[i]
    if (s.kind === 'round' && s.state === 'closed') return s
  }
  return null
}

/**
 * The Round before the Round at `pos`.
 *
 * Positions alternate Round, Gap, Round, Gap …, so the previous Round of a Round at
 * `pos` is at `pos - 2`. Only ever call this with a *Round's* position: `pos - 2`
 * from a Gap lands on another Gap, which is the live bug the contract warns about.
 */
export function previousRoundOf(
  segments: readonly Segment[],
  pos: number,
): RoundSegment | null {
  const prev = segments[pos - 2]
  return prev !== undefined && prev.kind === 'round' ? prev : null
}

/** The approval the Round at `pos` builds on: the previous Round's closing commit. */
export function previousApprovalOf(
  segments: readonly Segment[],
  pos: number,
): string | null {
  return previousRoundOf(segments, pos)?.closing_commit ?? null
}

/**
 * D4/U2: why a segment could not be placed, phrased for the rail.
 *
 * **These strings are copied verbatim from `UnplaceableReason::describe()`
 * (`src/round.rs`) and must stay that way.** The backend's wording is the one on the
 * wire — a repair's `skipped_reason` carries it, and `ghqc issue status` prints it — so
 * a card rendering this map next to a modal rendering `skipped_reason` would otherwise
 * explain one degradation two ways in a single view. Pinned by
 * `round::tests::every_reason_wording_matches_the_ui_copy` in `src/round.rs`, which
 * reads this file.
 */
export function unplaceableReasonText(reason: UnplaceableReason): string {
  switch (reason) {
    case 'branch_not_declared':
      return 'its round comment declared no branch'
    case 'branch_unavailable':
      return 'its branch is unavailable locally'
    case 'anchor_unreachable':
      return 'its commits are not on that branch'
    case 'merge_base_unreachable':
      return 'the histories it spans meet before Initial QC'
    case 'neighbour_unplaceable':
      return 'the round bounding it could not be placed'
  }
}

/**
 * Index of `hash` in an oldest-first commit list, or -1.
 *
 * Tolerates abbreviated hashes in either direction: the API mixes full 40-char
 * hashes (commit lists) with whatever length was recorded in a comment.
 */
export function findCommitIndex(
  ordered: readonly { hash: string }[],
  hash: string | null | undefined,
): number {
  if (!hash) return -1
  const exact = ordered.findIndex((c) => c.hash === hash)
  if (exact >= 0) return exact
  if (hash.length < 7) return -1
  return ordered.findIndex(
    (c) => c.hash.startsWith(hash) || (c.hash.length >= 7 && hash.startsWith(c.hash)),
  )
}

/**
 * The all-zero sha — git's "no object".
 *
 * `RoundSegment.opened_at` is pinned non-nullable on the wire, so a round whose anchor
 * resolved nowhere carries this instead of a null. Such a segment is always
 * `unplaceable`, but treating the value as a commit anywhere would print `0000000` as
 * if it were one, which is worse than printing nothing.
 */
export function isNullSha(hash: string | null | undefined): boolean {
  return !!hash && /^0+$/.test(hash)
}

export function shortHash(hash: string | null | undefined, len = 7): string {
  if (!hash || isNullSha(hash)) return '—'
  return hash.slice(0, len)
}
