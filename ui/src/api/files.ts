export type TreeEntryKind = 'file' | 'directory'

export interface TreeEntry {
  name: string
  kind: TreeEntryKind
}

export interface FileTreeResponse {
  path: string
  entries: TreeEntry[]
}

export async function fetchFileTree(path: string): Promise<FileTreeResponse> {
  const res = await fetch(`/api/files/tree?path=${encodeURIComponent(path)}`)
  if (!res.ok) throw new Error(`Failed to fetch file tree: ${res.status}`)
  return res.json()
}
