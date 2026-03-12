import { useQuery } from '@tanstack/react-query'
import { API_BASE } from '../config'

export interface Assignee {
  login: string
  name: string
}

async function fetchAssignees(): Promise<Assignee[]> {
  const res = await fetch(`${API_BASE}/assignees`)
  if (!res.ok) throw new Error(`Failed to fetch assignees: ${res.status}`)
  return res.json()
}

export function useAssignees() {
  return useQuery({
    queryKey: ['assignees'],
    queryFn: fetchAssignees,
  })
}
