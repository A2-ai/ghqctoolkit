// Client-side derivation of round membership and draft gaps.
//
// The API returns `rounds` oldest-first and `commits` newest-first. Everything
// in here works on an *oldest-first* commit list (`orderedCommits` in the UI),
// because that is the order the commit pickers index into.

import type { RoundInfo } from '~/api/rounds'

/** The commit shape the pickers work with (a subset of `IssueStatusResponse.commits`). */
export interface OrderedCommit {
  hash: string
  message: string
  statuses: string[]
  file_changed: boolean
}

/** Reverses the API's newest-first commit list into the oldest-first list the pickers use. */
export function toOrderedCommits<T>(commits: readonly T[]): T[] {
  return [...commits].reverse()
}

/**
 * Index of `hash` in an oldest-first commit list, or -1.
 *
 * Tolerates abbreviated hashes in either direction: the API mixes full 40-char
 * hashes (commit list) with whatever length was recorded in a comment.
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
 * A round's span over the oldest-first commit list.
 *
 * `startIdx..endIdx` is the round's *membership*: the commits after the anchor
 * (exclusive) through the closing commit, or through the newest commit while the
 * round is open.
 *
 * `anchorIdx` — the `opened_at` commit — is deliberately kept separate. It is not
 * a member of the round, but it *is* what the round's work is compared against,
 * so the pickers offer `anchorIdx..endIdx` as their selectable window. For a
 * legacy single `Initial QC` round that window is the whole history, which is
 * why scoping does not regress legacy issues.
 */
export interface RoundWindow {
  round: RoundInfo
  anchorIdx: number
  startIdx: number
  endIdx: number
}

/** Resolves one round's span, or null when its anchor is not in the commit list. */
export function roundWindow(
  ordered: readonly { hash: string }[],
  round: RoundInfo,
  /** Anchor index of the round that follows this one, used as a fallback end bound. */
  nextAnchorIdx: number | null = null,
): RoundWindow | null {
  const anchorIdx = findCommitIndex(ordered, round.opened_at)
  if (anchorIdx < 0) return null

  let endIdx: number
  if (round.state === 'closed') {
    const closing = findCommitIndex(ordered, round.closing_commit)
    endIdx = closing >= 0 ? closing : nextAnchorIdx !== null ? nextAnchorIdx - 1 : ordered.length - 1
  } else {
    endIdx = ordered.length - 1
  }
  if (endIdx < anchorIdx) endIdx = anchorIdx

  return { round, anchorIdx, startIdx: anchorIdx + 1, endIdx }
}

/** Resolves every round's span, dropping rounds whose anchor is missing. */
export function roundWindows(
  ordered: readonly { hash: string }[],
  rounds: readonly RoundInfo[],
): RoundWindow[] {
  const anchors = rounds.map((r) => findCommitIndex(ordered, r.opened_at))
  const out: RoundWindow[] = []
  rounds.forEach((round, i) => {
    const nextAnchor = anchors.slice(i + 1).find((a) => a >= 0)
    const w = roundWindow(ordered, round, nextAnchor ?? null)
    if (w) out.push(w)
  })
  return out
}

/** The span of the currently open round, or null when nothing is open / resolvable. */
export function openRoundWindow(
  ordered: readonly { hash: string }[],
  rounds: readonly RoundInfo[],
  openRoundIndex: number | null,
): RoundWindow | null {
  if (openRoundIndex === null) return null
  const idx = rounds.findIndex((r) => r.index === openRoundIndex)
  if (idx < 0) return null
  return roundWindow(ordered, rounds[idx], null)
}

/**
 * Runs of commits that belong to no round — the "draft gap" a user creates by
 * editing after an approval and before opening the next round.
 *
 * Only runs *between* two rounds are reported: commits before the first round's
 * anchor are history the rounds never claimed, and hiding those behind a marker
 * would be surprising rather than helpful.
 */
export function draftGapRuns(
  ordered: readonly { hash: string }[],
  rounds: readonly RoundInfo[],
): number[][] {
  const windows = roundWindows(ordered, rounds)
  if (windows.length < 2) return []

  const covered = new Set<number>()
  for (const w of windows) {
    for (let i = w.anchorIdx; i <= w.endIdx; i++) covered.add(i)
  }

  const lo = Math.min(...windows.map((w) => w.anchorIdx))
  const hi = Math.max(...windows.map((w) => w.endIdx))

  const runs: number[][] = []
  let current: number[] = []
  for (let i = lo; i <= hi; i++) {
    if (covered.has(i)) {
      if (current.length > 0) runs.push(current)
      current = []
    } else {
      current.push(i)
    }
  }
  // A trailing run would extend past `hi`, which cannot happen: `hi` is covered.
  return runs
}

/** `"Round 2"` → the previous round's display name, for "Since <name> approval" labels. */
export function previousRoundName(rounds: readonly RoundInfo[], round: RoundInfo): string | null {
  const prev = rounds.filter((r) => r.index < round.index).pop()
  return prev?.name ?? null
}

export function shortHash(hash: string | null | undefined, len = 7): string {
  return hash ? hash.slice(0, len) : '—'
}
