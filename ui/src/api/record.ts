export interface RecordContextFileRequest {
  server_path: string
  position: 'prepend' | 'append'
}

export interface RecordRequest {
  milestone_numbers: number[]
  tables_only: boolean
  output_path: string
  context_files: RecordContextFileRequest[]
}

export async function uploadContextFile(file: File): Promise<{ temp_path: string }> {
  const form = new FormData()
  form.append('file', file)
  const res = await fetch('/api/record/upload', { method: 'POST', body: form })
  if (!res.ok) {
    const text = await res.text()
    throw new Error(`Upload failed: ${text}`)
  }
  return res.json()
}

export async function previewRecord(req: RecordRequest): Promise<{ key: string }> {
  const res = await fetch('/api/record/preview', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(req),
  })
  if (!res.ok) {
    const text = await res.text()
    throw new Error(`Preview failed: ${text}`)
  }
  return res.json()
}

export async function generateRecord(req: RecordRequest): Promise<void> {
  const res = await fetch('/api/record/generate', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(req),
  })
  if (!res.ok) {
    const text = await res.text()
    throw new Error(`Generate failed: ${text}`)
  }
}
