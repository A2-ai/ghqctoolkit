import { useState } from 'react'
import { Card, Stack, Text, Title, Tooltip } from '@mantine/core'
import { DragDropContext, Droppable, Draggable, DropResult } from '@hello-pangea/dnd'
import type { IssueStatusResponse, QCStatus } from '~/api/issues'
import { latestRound } from '~/api/issues'
import { IssueCard } from './IssueCard'
import { IssueDetailModal } from './IssueDetailModal'
import { NewRoundModal } from './NewRoundModal'

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

// U7/D35: the ChangesAfterApproval hash comes from `drift.newest_file_change` — the
// server's single source for it. Rescanning `drift.commits` here is exactly the
// client-side re-derivation U7 forbids. Dispatch on the latest round's state first
// (S0): an open round's drift is also empty, so emptiness means nothing on its own.
function postApprovalFileCommit(s: IssueStatusResponse): string | undefined {
  if (latestRound(s).state.kind !== 'approved') return undefined
  return s.drift.newest_file_change ?? undefined
}

export function SwimLanes({ statuses, currentBranch, remoteCommit }: Props) {
  const [selected, setSelected] = useState<IssueStatusResponse | null>(null)
  // Owned here, not by the card: a Modal is portaled in the DOM but still bubbles
  // React events up its element tree, so a modal rendered inside the clickable card
  // would re-open the detail modal on every click inside it.
  const [newRoundFor, setNewRoundFor] = useState<IssueStatusResponse | null>(null)

  const byLane: Record<string, IssueStatusResponse[]> = Object.fromEntries(
    LANES.map((l) => [l.id, []])
  )
  for (const s of statuses) {
    byLane[getLaneId(s.qc_status.status)].push(s)
  }

  return (
    <>
    <DragDropContext onDragEnd={noop}>
      <div style={{ display: 'flex', gap: 12, alignItems: 'stretch', height: '100%', minHeight: 0, overflowX: 'auto', overflowY: 'hidden' }}>
        {LANES.map((lane) => {
          const cards = byLane[lane.id]
          return (
            <div key={lane.id} style={{ flex: 1, minWidth: 220, minHeight: 0, display: 'flex' }}>
              <Card withBorder style={{ display: 'flex', flexDirection: 'column', flex: 1, minHeight: 0 }}>
                <Stack style={{ flex: 1, minHeight: 0 }} gap="sm">
                  <div style={{ background: lane.headerColor, padding: '6px 8px', borderRadius: 4 }}>
                    <Title order={5} style={{ textAlign: 'center' }}>{lane.title}</Title>
                  </div>
                  <Droppable droppableId={lane.id}>
                    {(provided) => (
                      <div
                        ref={provided.innerRef}
                        {...provided.droppableProps}
                        style={{ flex: 1, minHeight: 0, overflowY: 'auto', paddingRight: 4 }}
                      >
                        {cards.map((s, index) => {
                          const postApprovalCommit = postApprovalFileCommit(s)
                          const colorTooltip =
                            s.qc_status.status === 'approval_required'
                              ? 'Issue was closed without approval'
                              : postApprovalCommit
                              ? 'File has changed since approval'
                              : null
                          return (
                          <Draggable
                            key={s.issue.number}
                            draggableId={String(s.issue.number)}
                            index={index}
                            isDragDisabled
                          >
                            {(p) => {
                              const card = (
                                <Card
                                  ref={p.innerRef}
                                  {...p.draggableProps}
                                  {...p.dragHandleProps}
                                  withBorder
                                  mb={8}
                                  p={10}
                                  onClick={() => setSelected(s)}
                                  data-testid={`issue-card-${s.issue.number}`}
                                  style={{
                                    cursor: 'pointer',
                                    ...(s.qc_status.status === 'approval_required'
                                      ? { backgroundColor: '#fee2e2' }
                                      : postApprovalCommit
                                      ? { backgroundColor: '#ffedd5' }
                                      : undefined),
                                  }}
                                >
                                  <IssueCard
                                    status={s}
                                    currentBranch={currentBranch}
                                    remoteCommit={remoteCommit}
                                    postApprovalCommit={postApprovalCommit}
                                    onNewRound={() => setNewRoundFor(s)}
                                  />
                                </Card>
                              )
                              return colorTooltip ? (
                                <Tooltip label={colorTooltip} withArrow position="top" openDelay={300}>
                                  {card}
                                </Tooltip>
                              ) : card
                            }}
                          </Draggable>
                          )
                        })}
                        {cards.length === 0 && (
                          <Text c="dimmed" size="sm" style={{ textAlign: 'center', paddingTop: 8 }}>
                            Empty
                          </Text>
                        )}
                        {provided.placeholder}
                      </div>
                    )}
                  </Droppable>
                </Stack>
              </Card>
            </div>
          )
        })}
      </div>
    </DragDropContext>
    <IssueDetailModal status={selected} onClose={() => setSelected(null)} onStatusUpdate={setSelected} />
    {newRoundFor && (
      <NewRoundModal opened onClose={() => setNewRoundFor(null)} status={newRoundFor} />
    )}
    </>
  )
}
