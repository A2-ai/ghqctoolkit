import { useState } from 'react'
import { Card, Stack, Text, Title } from '@mantine/core'
import { DragDropContext, Droppable, Draggable, DropResult } from '@hello-pangea/dnd'
import type { IssueStatusResponse, QCStatus } from '~/api/issues'
import { IssueCard } from './IssueCard'
import { IssueDetailModal } from './IssueDetailModal'

const LANES: { id: string; title: string; headerColor: string }[] = [
  { id: 'ready-for-review',    title: 'Ready for Review',    headerColor: '#dbeafe' },
  { id: 'findings-to-address', title: 'Findings to Address', headerColor: '#fee2e2' },
  { id: 'changes-to-notify',   title: 'Changes to Notify',  headerColor: '#fef9c3' },
  { id: 'approved',            title: 'Approved',            headerColor: '#dcfce7' },
]

function getLaneId(status: QCStatus['status']): string {
  switch (status) {
    case 'approved':
    case 'changes_after_approval':
      return 'approved'
    case 'awaiting_review':
    case 'approval_required':
      return 'ready-for-review'
    case 'change_requested':
      return 'findings-to-address'
    case 'in_progress':
    case 'changes_to_comment':
      return 'changes-to-notify'
  }
}

function noop(_: DropResult) {}

interface Props {
  statuses: IssueStatusResponse[]
  currentBranch: string
  remoteCommit: string
}

// Commits are newest-first; commits before the approved index are temporally later.
function postApprovalFileCommit(s: IssueStatusResponse): string | undefined {
  const { approved_commit } = s.qc_status
  if (!approved_commit) return undefined
  const approvedIdx = s.commits.findIndex((c) => c.hash === approved_commit)
  if (approvedIdx <= 0) return undefined
  return s.commits.slice(0, approvedIdx).find((c) => c.file_changed)?.hash
}

export function SwimLanes({ statuses, currentBranch, remoteCommit }: Props) {
  const [selected, setSelected] = useState<IssueStatusResponse | null>(null)

  const byLane: Record<string, IssueStatusResponse[]> = Object.fromEntries(
    LANES.map((l) => [l.id, []])
  )
  for (const s of statuses) {
    byLane[getLaneId(s.qc_status.status)].push(s)
  }

  return (
    <>
    <DragDropContext onDragEnd={noop}>
      <div style={{ display: 'flex', gap: 12, alignItems: 'flex-start' }}>
        {LANES.map((lane) => {
          const cards = byLane[lane.id]
          return (
            <div key={lane.id} style={{ flex: 1, minWidth: 180 }}>
              <Card withBorder>
                <Stack>
                  <div style={{ background: lane.headerColor, padding: '6px 8px', borderRadius: 4 }}>
                    <Title order={5} style={{ textAlign: 'center' }}>{lane.title}</Title>
                  </div>
                  <Droppable droppableId={lane.id}>
                    {(provided) => (
                      <div
                        ref={provided.innerRef}
                        {...provided.droppableProps}
                        style={{ minHeight: 120 }}
                      >
                        {cards.map((s, index) => (
                          <Draggable
                            key={s.issue.number}
                            draggableId={String(s.issue.number)}
                            index={index}
                            isDragDisabled
                          >
                            {(p) => (
                              <Card
                                ref={p.innerRef}
                                {...p.draggableProps}
                                {...p.dragHandleProps}
                                withBorder
                                mb={8}
                                p={10}
                                onClick={() => setSelected(s)}
                                style={{
                                  cursor: 'pointer',
                                  ...(postApprovalFileCommit(s) ? { backgroundColor: '#ffedd5' } : undefined),
                                }}
                              >
                                <IssueCard status={s} currentBranch={currentBranch} remoteCommit={remoteCommit} postApprovalCommit={postApprovalFileCommit(s)} />
                              </Card>
                            )}
                          </Draggable>
                        ))}
                        {provided.placeholder}
                      </div>
                    )}
                  </Droppable>
                  {cards.length === 0 && (
                    <Text c="dimmed" size="sm" style={{ textAlign: 'center' }}>Empty</Text>
                  )}
                </Stack>
              </Card>
            </div>
          )
        })}
      </div>
    </DragDropContext>
    <IssueDetailModal status={selected} onClose={() => setSelected(null)} onStatusUpdate={setSelected} />
    </>
  )
}
