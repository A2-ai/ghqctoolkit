import { useQuery } from '@tanstack/react-query'

export interface Milestone {
  number: number
  title: string
  state: 'open' | 'closed'
  description: string | null
  open_issues: number
  closed_issues: number
}

async function fetchMilestones(): Promise<Milestone[]> {
  const res = await fetch('/api/milestones')
  if (!res.ok) throw new Error(`Failed to fetch milestones: ${res.status}`)
  return res.json()
}

export function useMilestones() {
  return useQuery({
    queryKey: ['milestones'],
    queryFn: fetchMilestones,
  })
}
