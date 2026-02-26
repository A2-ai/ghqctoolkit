export interface ArchiveFileRequest {
  repository_file: string
  commit: string
  milestone?: string
  approved?: boolean
}

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
  const res = await fetch('/api/archive/generate', {
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
