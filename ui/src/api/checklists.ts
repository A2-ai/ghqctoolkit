export interface Checklist {
  name: string
  content: string
}

export async function fetchChecklists(): Promise<Checklist[]> {
  const res = await fetch('/api/configuration')
  if (!res.ok) throw new Error(`Failed to fetch checklists: ${res.status}`)
  const data = await res.json()
  return data.checklists
}
