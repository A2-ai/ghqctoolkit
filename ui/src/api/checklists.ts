export interface Checklist {
  name: string
  content: string
}

export async function fetchChecklists(): Promise<Checklist[]> {
  const res = await fetch('/api/configuration/checklists')
  if (!res.ok) throw new Error(`Failed to fetch checklists: ${res.status}`)
  return res.json()
}
