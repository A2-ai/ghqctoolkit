import type { QueuedItem, RelevantFileDraft } from '~/components/CreateIssueModal'
import type { Milestone } from './milestones'

// ── Request types ─────────────────────────────────────────────────────────────

type RelevantIssueClass =
  | { Exists: { issue_number: number } }
  | { New: string }

interface RelevantIssue {
  file_name: string
  issue_class: RelevantIssueClass
  description?: string | null
}

interface RelevantFileInput {
  file_path: string
  justification: string
}

export interface CreateIssueRequest {
  file: string
  checklist_name: string
  checklist_content: string
  assignees?: string[]
  gating_qc?: RelevantIssue[]
  previous_qc?: RelevantIssue[]
  relevant_qc?: RelevantIssue[]
  relevant_files?: RelevantFileInput[]
}

// ── Response types ────────────────────────────────────────────────────────────

interface BlockingQCError {
  issue_number: number
  error: string
}

export interface CreateIssueResponse {
  issue_url: string
  issue_number: number
  blocking_created: number[]
  blocking_errors: BlockingQCError[]
}

// ── Conversion ────────────────────────────────────────────────────────────────

function toRelevantIssue(rf: RelevantFileDraft, batchFiles: Set<string>): RelevantIssue | null {
  let issueClass: RelevantIssueClass
  if (rf.issueNumber !== null) {
    issueClass = { Exists: { issue_number: rf.issueNumber } }
  } else if (batchFiles.has(rf.file)) {
    issueClass = { New: rf.file }
  } else {
    return null // queued item no longer in batch — skip
  }
  return {
    file_name: rf.file,
    issue_class: issueClass,
    description: rf.description || null,
  }
}

export function toCreateIssueRequest(item: QueuedItem, batchFiles: Set<string>): CreateIssueRequest {
  const gatingQc: RelevantIssue[] = []
  const relevantQc: RelevantIssue[] = []
  const relevantFiles: RelevantFileInput[] = []

  for (const rf of item.relevantFiles) {
    if (rf.kind === 'file') {
      relevantFiles.push({ file_path: rf.file, justification: rf.description })
    } else {
      const ri = toRelevantIssue(rf, batchFiles)
      if (!ri) continue
      if (rf.kind === 'blocking_qc') {
        gatingQc.push(ri)
      } else {
        relevantQc.push(ri)
      }
    }
  }

  return {
    file: item.file,
    checklist_name: item.checklistName,
    checklist_content: item.checklistContent,
    ...(item.assignees.length > 0 && { assignees: item.assignees }),
    ...(gatingQc.length > 0 && { gating_qc: gatingQc }),
    ...(relevantQc.length > 0 && { relevant_qc: relevantQc }),
    ...(relevantFiles.length > 0 && { relevant_files: relevantFiles }),
  }
}

// ── API calls ─────────────────────────────────────────────────────────────────

async function parseError(res: Response, fallback: string): Promise<string> {
  try {
    const data = await res.json()
    if (typeof data.error === 'string') return data.error
    if (typeof data === 'string') return data
  } catch {}
  return `${fallback}: ${res.status}`
}

export async function postCreateMilestone(name: string, description: string | null): Promise<Milestone> {
  const res = await fetch('/api/milestones', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ name, description }),
  })
  if (!res.ok) throw new Error(await parseError(res, 'Failed to create milestone'))
  return res.json()
}

export async function postCreateIssues(
  milestoneNumber: number,
  requests: CreateIssueRequest[],
): Promise<CreateIssueResponse[]> {
  const res = await fetch(`/api/milestones/${milestoneNumber}/issues`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(requests),
  })
  if (!res.ok) throw new Error(await parseError(res, 'Failed to create issues'))
  return res.json()
}
