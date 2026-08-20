// What the archive would take for one milestone file — read off the status response, never
// derived from it.
//
// **The rules live in one place, and it is not this file.** `RoundSegment.archive_preview`
// is the backend projecting the archive's own derivation per round: the commit S1's three
// rows select, the approval I2/§13.1 attributes to it, and every supersession clause S3
// finds. This file used to reimplement all three in TypeScript. That was one rule in two
// languages — the drift §18.4 forbids and §25.1 escalated — and §26 closed it by projecting
// the answer. So there is no `closing_commit` versus `latest_actioned_commit` choice here, no
// approval tie-break, and no clause list: the preview is rendered.
//
// What legitimately stays: which round a selection addresses (the user's choice plus D9's
// default), the U4 filter predicates, U5's aggregation, and all wording.
//
// Two shapes to keep straight, both by decision:
//
// - `archive_preview === null` means **this round cannot be archived** (§18.1/§20.2), which
//   is what U8 blocks on. It never means "unknown round" — the segment is right there — and
//   the reason for the refusal is on `placement.reason`, not inside the preview.
// - There is no `superseded` bool on the wire (§26.3). `superseded` **is**
//   `superseding_causes.length > 0`, so nothing here stores a synthesized bool to branch on
//   twice; callers branch on the array, or on `isApprovedAndCurrent`.

import type { IssueStatusResponse } from '~/api/issues'
import type {
  ArchivePreview,
  PreviewApproval,
  RoundSegment,
  SupersedingCause,
  UnplaceableReason,
} from '~/api/rounds'
import { activeRound, roundSegments, shortHash } from '~/utils/rounds'

/** Where a file's archive selection landed, and what the wire says about it. */
export interface ArchiveSelection {
  /** Every round of the thread, oldest first. */
  rounds: RoundSegment[]
  /** The round the selection addresses. Null only when the thread carries no round. */
  selected: RoundSegment | null
  /** True when the user retargeted away from the latest round (D7) — an override (U2). */
  isOverride: boolean
  /**
   * The projected answer for the selected round, straight off the wire. Null ⇒ that round
   * cannot be archived, and `blocked` carries what to say about it.
   */
  preview: ArchivePreview | null
  /** `preview.commit` — the commit the archive would take. Null when blocked. */
  commit: string | null
  /** `preview.approval` — null when those bytes were never approved. */
  approval: PreviewApproval | null
  /**
   * `preview.superseding_causes`, verbatim and in clause order. Empty is a **positive**
   * claim of currency, not an absence.
   */
  causes: SupersedingCause[]
  /** Those causes in the user's terms, in the same order. Empty when `causes` is. */
  stale: string[]
  /** Set when the selected round cannot be archived (§18.1/§20.2). */
  blocked: { roundName: string; reason: UnplaceableReason | null } | null
}

/**
 * The round a selection addresses: the override when the user set one, else the latest
 * round (D9/S2).
 *
 * An override naming a round the thread does not have is ignored rather than sent — the
 * server would reject it by number, and a stale override survives a thread that gained or
 * lost rounds between renders.
 */
function selectedRoundOf(rounds: RoundSegment[], override: number | undefined): RoundSegment | null {
  if (rounds.length === 0) return null
  if (override !== undefined) {
    const match = rounds.find((round) => round.index === override)
    if (match) return match
  }
  return rounds[rounds.length - 1]
}

/**
 * What the archive would take for `status`, at `override` or at the latest round.
 *
 * Pure, and a read throughout: it picks a round and hands back that round's own preview.
 */
export function archiveSelectionOf(
  status: IssueStatusResponse,
  override?: number,
): ArchiveSelection {
  const segments = status.segments ?? []
  const rounds = roundSegments(segments)
  const selected = selectedRoundOf(rounds, override)
  const latest = rounds.length > 0 ? rounds[rounds.length - 1] : null

  if (selected === null || latest === null) {
    return {
      rounds,
      selected: null,
      isOverride: false,
      preview: null,
      commit: null,
      approval: null,
      causes: [],
      stale: [],
      blocked: null,
    }
  }

  const isOverride = selected.index !== latest.index
  const preview = selected.archive_preview

  // The block signal is the preview's absence, which is the archive's own refusal — not a
  // second reading of `placement`, and emphatically not `closing_commit === null`: a round
  // can close at a real sha and still be unarchivable (§20.2).
  if (preview === null) {
    return {
      rounds,
      selected,
      isOverride,
      preview: null,
      commit: null,
      approval: null,
      causes: [],
      stale: [],
      blocked: {
        roundName: selected.name,
        // The reason lives on `placement`, and only there. Null is unreachable on a
        // conforming response (null preview ⟺ unplaceable) and is rendered as an
        // unexplained refusal rather than crashing on a shape the server says cannot occur.
        reason: selected.placement.kind === 'unplaceable' ? selected.placement.reason : null,
      },
    }
  }

  const causes = preview.superseding_causes
  return {
    rounds,
    selected,
    isOverride,
    preview,
    commit: preview.commit,
    approval: preview.approval,
    causes,
    stale: causes.map((cause) => causeText(cause, selected, latest)),
    blocked: null,
  }
}

/**
 * One supersession cause in the user's terms (U1) — presentation over a cause the server
 * already decided, never a re-test of the clause that produced it.
 *
 * The round names come from the segments positionally, which is why the wording can name
 * *which* round is open without asking whether one is.
 */
function causeText(
  cause: SupersedingCause,
  selected: RoundSegment,
  latest: RoundSegment,
): string {
  switch (cause) {
    case 'later_approval':
      return 'a later round has since approved'
    case 'round_open':
      // Same cause, two readings: when the previewed round *is* the open one, "it is open"
      // understates what matters — the bytes are mid-review.
      return selected.index === latest.index && selected.state === 'open'
        ? `${selected.name} is open — these bytes are under review, not approved`
        : `${latest.name} is open`
    case 'changed_since':
      return 'the file changed after this approval'
    case 'undeterminable':
      // §28.2: half of this clause is an `Unrelated` gap, which is *placed* and perfectly
      // readable — it simply spans histories with no common ancestor, so no range between
      // its bounds is meaningful. "Could not be read" describes only the other half.
      return `what happened after ${selected.name} could not be located, or spans histories with no common ancestor`
  }
}

/**
 * True when the bytes are an approval that nothing newer has overtaken.
 *
 * `superseded ⟺ causes.length > 0` (§26.3) — the bool is derived at the one place that
 * needs it rather than stored beside the array it comes from.
 */
export function isApprovedAndCurrent(selection: ArchiveSelection): boolean {
  return selection.approval !== null && selection.causes.length === 0
}

/**
 * One file's provenance, as one line: which round the selection addressed, whether those
 * bytes were approved, by whom and when, and the commit (U1/D1).
 *
 * **The two round frames stay apart.** Nothing here claims the *selected* round was
 * approved — only that the commit taken is some round's approval — because `approval.round`
 * may be less than the round being previewed (I2), and collapsing the two frames into one
 * "Round 3 · approved" is how approved content came to be labelled unapproved in the first
 * place. Wording tracks `provenance_line` in `src/cli/archive.rs` so a user reading the GUI
 * card and the CLI's pre-archive report is told the same thing.
 */
export function provenanceLine(selection: ArchiveSelection): string {
  const { selected, approval, commit } = selection
  if (selected === null) return '—'
  if (selection.blocked !== null) {
    return `${selected.name} · cannot be archived`
  }
  const when = approval?.at ? formatDay(approval.at) : null
  const who = approval?.by ? `@${approval.by}` : 'an unknown reviewer'
  const bytes =
    approval === null
      ? `unapproved · ${shortHash(commit)}`
      : approval.round === selected.index
        ? `approved by ${who}${when ? ` on ${when}` : ''} · ${shortHash(commit)}`
        : `bytes are ${roundName(selection.rounds, approval.round)}'s approval by ${who}${when ? ` on ${when}` : ''} · ${shortHash(commit)}`
  return `${selected.name} · ${bytes}`
}

/**
 * The display name of the round at `index`, looked up positionally in the segments the
 * response already carried. The fallback is only for a response naming a round it did not
 * send, which the wire does not do.
 */
function roundName(rounds: RoundSegment[], index: number): string {
  return rounds.find((round) => round.index === index)?.name ?? `Round ${index}`
}

/**
 * One round, described well enough to choose it (U2): which round, open or closed, the
 * commit the archive would take for it, and who approved those bytes.
 *
 * Every fact comes from that round's **own** preview, so the description of an option and
 * the provenance shown once it is chosen are the same answer rather than two that agree. A
 * round with no preview cannot be archived and says so — labelled, not hidden: the choice
 * stays the user's, and choosing it is refused with the same wording.
 */
export function describeRound(round: RoundSegment): string {
  const preview = round.archive_preview
  if (preview === null) {
    return `${round.name} · cannot be archived`
  }
  if (round.state === 'closed') {
    const when = round.closed_at ? formatDay(round.closed_at) : null
    return `${round.name} · closed · approved by @${preview.approval?.by ?? round.closed_by ?? '?'}${when ? ` on ${when}` : ''} · ${shortHash(preview.commit)}`
  }
  // An open round archives the newest commit somebody *acted on* — its anchor, a
  // notification or a review — which is what the preview's commit already is, not the
  // newest commit on the branch (S6).
  return `${round.name} · open · would archive ${shortHash(preview.commit)}`
}

/**
 * S4, in the current-state reading (§17.2): a thread where **no round has ever closed**.
 *
 * This is the whole of what "include non-approved" now governs. It is not one of the
 * deleted approval predicates and not a supersession rule: it never picks a commit, never
 * labels bytes, and is asked of the rounds' own `closing_commit` rather than of
 * `qc_status.status` or the GitHub issue state. A retraction reopens the round and clears
 * its closing commit, so a withdrawn approval counts as never approved here — the same
 * reading the server's provenance takes.
 */
export function neverApproved(status: IssueStatusResponse): boolean {
  return roundSegments(status.segments ?? []).every((round) => round.closing_commit === null)
}

// ─── U4: round-aware bulk filters ────────────────────────────────────────────
//
// Presentation, explicitly sanctioned as staying here (§26.6): each is a read of a segment
// field, of a field `qc_status` already carries, or of the projected preview. None of them
// re-tests a supersession clause — "changed since approval" asks the server's own
// `changed_commit` (the newest file-changing commit of the trailing gap), and "unplaceable"
// asks the gate's own subject, the **selected** round's preview.

export type ArchiveFilterKey =
  | 'under_review'
  | 'changed_since_approval'
  | 'approved_round_2_plus'
  | 'never_approved'
  | 'unplaceable'

export const ARCHIVE_FILTERS: { key: ArchiveFilterKey; label: string }[] = [
  { key: 'under_review', label: 'Under review' },
  { key: 'changed_since_approval', label: 'Changed since approval' },
  { key: 'approved_round_2_plus', label: 'Approved in round ≥ 2' },
  { key: 'never_approved', label: 'Never approved' },
  { key: 'unplaceable', label: 'Unplaceable' },
]

export function matchesArchiveFilter(
  key: ArchiveFilterKey,
  status: IssueStatusResponse,
  selection: ArchiveSelection,
): boolean {
  switch (key) {
    case 'under_review':
      return activeRound(status.segments ?? []) !== null
    case 'changed_since_approval':
      return status.qc_status.changed_commit !== null
    case 'approved_round_2_plus':
      return selection.rounds.some((round) => round.index >= 2 && round.closing_commit !== null)
    case 'never_approved':
      return neverApproved(status)
    case 'unplaceable':
      return selection.blocked !== null
  }
}

function formatDay(iso: string): string {
  const d = new Date(iso)
  return Number.isNaN(d.getTime()) ? iso : d.toISOString().slice(0, 10)
}
