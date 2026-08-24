import { API_BASE } from '../config'

/**
 * A6/D28.3: a QC-attached file carries **all four** frozen facts or none of them —
 * the backend rejects any partial quartet with a 400, because only the client knows
 * which round it selected (U5) and there is no safe default to invent for an audit
 * snapshot.
 */
export interface ArchiveFileRequest {
  repository_file: string
  commit: string
  milestone?: string
  approved?: boolean
  /** The round the selected commit belongs to (D15). */
  round?: number
  /** Whether file-changing commits exist after the selected commit (R11/R12). */
  subsequent_file_changes?: boolean
}

/**
 * D62/D61: a file whose selected round has no resolvable commit is **omitted** from
 * `files` and declared here, so a partial archive says so in its own manifest — the
 * server writes these verbatim into `ghqc_archive_metadata.json`'s `skipped` list.
 *
 * Skipping is not falling back: the alternative would be freezing a substitute
 * commit, which is exactly the audit lie §18 removes.
 */
export interface SkippedFileRequest {
  repository_file: string
  round: number
  branch: string
  reason: string
}

export interface ArchiveGenerateRequest {
  output_path: string
  flatten: boolean
  files: ArchiveFileRequest[]
  /** D62: `#[serde(default)]` server-side, so omitting it is the complete-archive case. */
  skipped?: SkippedFileRequest[]
}

export interface ArchiveGenerateResponse {
  output_path: string
  /**
   * D62: the server echoes what it recorded, so a client can confirm the omission
   * reached the archive's metadata.
   *
   * Deliberately **not** optional, and deliberately asymmetric with the request and
   * with the manifest: the manifest uses `skip_serializing_if` so a complete archive
   * keeps its old byte-for-byte shape, but the response always carries the field and
   * sends `[]` when nothing was skipped. An optional type here would let a stale mock
   * omit it and still compile — the fixture drift D63 exists to catch.
   */
  skipped: SkippedFileRequest[]
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
    const data = await res.json().catch(() => null)
    throw new Error(data?.error ?? `Failed to generate archive: ${res.status}`)
  }
  return res.json()
}
