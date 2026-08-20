import { API_BASE } from '../config'

/**
 * One file to archive. The mode is **declared**, never inferred from which optional
 * fields happen to be present (contract §2.1): the server deserializes this into an
 * internally tagged enum with `deny_unknown_fields`, so a stale key is a rejection that
 * names the key rather than a value silently discarded.
 *
 * Do not hand-assemble a body against these types — a mixed entry carrying both a round
 * selection and a hand-picked commit is what the tag exists to forbid.
 */

/**
 * Mode 1 (A1): a milestone QC file. The issue number is the only handle the server
 * needs: it reads the thread and derives the path, the milestone, the commit, the
 * approval and whether the bytes were superseded from it (D6).
 */
export interface ArchiveIssueFileRequest {
  mode: 'issue'
  issue_number: number
  /**
   * The round this selection addresses, 1-based; `1` is Initial QC. `null` targets the
   * **latest** round (D9) — which for an approved-then-reopened file is unapproved
   * content, intentionally and ungated (S2).
   *
   * Not `?`-optional on purpose (contract §2.2): the server tolerates an absent key for
   * `curl` callers, but the UI has exactly one encoding of "latest", so a default always
   * travels as an explicit `null` and a number always means the user overrode it.
   */
  round: number | null
}

/** Mode 2 (D4): a file in no milestone — the user picks the commit directly. */
export interface ArchiveAddedFileRequest {
  mode: 'file'
  repository_file: string
  commit: string
}

export type ArchiveFileRequest = ArchiveIssueFileRequest | ArchiveAddedFileRequest

export interface ArchiveGenerateRequest {
  output_path: string
  flatten: boolean
  files: ArchiveFileRequest[]
}

export interface ArchiveGenerateResponse {
  output_path: string
}

export async function generateArchive(
  request: ArchiveGenerateRequest,
): Promise<ArchiveGenerateResponse> {
  const res = await fetch(`${API_BASE}/archive/generate`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(request),
  })
  if (!res.ok) {
    // Every client error on this route wears the `{"error": …}` envelope, including
    // axum's own body-shape rejection, which the handler normalizes into it. The
    // round-selection and unplaceable messages name the issue and the reason
    // deliberately, so the server's message is what the user sees; the status-only
    // fallback is for a body that is genuinely not JSON.
    const data = await res.json().catch(() => null)
    throw new Error(data?.error ?? `Failed to generate archive: ${res.status}`)
  }
  return res.json()
}
