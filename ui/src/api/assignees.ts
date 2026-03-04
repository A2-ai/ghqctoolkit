import { useQuery } from '@tanstack/react-query'

export interface Assignee {
  login: string
  name: string
}

async function fetchAssignees(): Promise<Assignee[]> {
  const res = await fetch('/api/assignees')
  if (!res.ok) throw new Error(`Failed to fetch assignees: ${res.status}`)
  return res.json()
}

export function useAssignees() {
  return useQuery({
    queryKey: ['assignees'],
    queryFn: fetchAssignees,
  })
}
