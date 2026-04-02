import { API_BASE } from '../config'

export type TreeEntryKind = 'file' | 'directory'

export interface TreeEntry {
  name: string
  kind: TreeEntryKind
}

export interface FileTreeResponse {
  path: string
  entries: TreeEntry[]
}

export interface FileCollaboratorsResponse {
  path: string
  author: string | null
  collaborators: string[]
}

export async function fetchFileTree(path: string): Promise<FileTreeResponse> {
  const res = await fetch(`${API_BASE}/files/tree?path=${encodeURIComponent(path)}`)
  if (!res.ok) throw new Error(`Failed to fetch file tree: ${res.status}`)
  return res.json()
}

export async function fetchFileCollaborators(path: string): Promise<FileCollaboratorsResponse> {
  const res = await fetch(`${API_BASE}/files/collaborators?path=${encodeURIComponent(path)}`)
  if (!res.ok) throw new Error(`Failed to fetch file collaborators: ${res.status}`)
  return res.json()
}
