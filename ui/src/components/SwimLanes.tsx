import { useState } from 'react'
import { Card, Stack, Text, Title, Tooltip } from '@mantine/core'
import { DragDropContext, Droppable, Draggable, DropResult } from '@hello-pangea/dnd'
import type { IssueStatusResponse, QCStatus } from '~/api/issues'
import { IssueCard } from './IssueCard'
import { IssueDetailModal } from './IssueDetailModal'
import { StartRoundModal } from './StartRoundModal'
import { lastClosedRound } from '~/utils/rounds'

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
    // `unknown` means the active segment could not be placed, so no lane is truly
    // right. This is where the state landed before it had its own status value (it
    // was reported as `in_progress`), so keeping it here changes no placement. The
    // card is grayed with its reason, which is what actually tells the user it is not
    // actionable. Whether it deserves a lane of its own is an open UX question.
    case 'unknown':
      return 'changes-to-notify'
  }
}

function noop(_: DropResult) {}

interface Props {
  statuses: IssueStatusResponse[]
  currentBranch: string
  remoteCommit: string
}

// `postApprovalFileCommit` lived here: a hand-rolled scan of the thread-wide commit
// list for the newest file change after the standing approval, gated on no round
// being open. S6 deletes it — both halves are now the backend's answer. S1 makes
// `changes_after_approval` mean exactly "the trailing Gap is non-empty", which can
// only hold while no round is open (I3), and `changed_commit` is the commit that
// status is *about*. What is left is a read of two fields at the call site below (D7).
//
// `changed_commit`, not `latest_commit`: S1 names the trailing Gap's newest
// *file-changing* commit, while `latest_commit` is its newest commit full stop. For a
// Gap whose newest drift never touched the file the two differ, and this row would
// otherwise name a commit that never touched it.

export function SwimLanes({ statuses, currentBranch, remoteCommit }: Props) {
  const [selected, setSelected] = useState<IssueStatusResponse | null>(null)
  // Issue the start-new-round modal is open for (S6 entry point).
  const [startRoundFor, setStartRoundFor] = useState<IssueStatusResponse | null>(null)

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
            // The lane container is addressable so a test can assert *which* lane a
            // card landed in. Scoping by "the element containing the lane heading"
            // matches <html> and <body> too, so such an assertion passes wherever the
            // card actually is.
            <div
              key={lane.id}
              data-testid={`lane-${lane.id}`}
              style={{ flex: 1, minWidth: 220, minHeight: 0, display: 'flex' }}
            >
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
                          const postApprovalCommit =
                            s.qc_status.status === 'changes_after_approval'
                              ? (s.qc_status.changed_commit ?? undefined)
                              : undefined
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
                                    onStartRound={() => setStartRoundFor(s)}
                                    onRepairRound={() => setStartRoundFor(s)}
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
    <IssueDetailModal
      status={selected}
      onClose={() => setSelected(null)}
      onStatusUpdate={setSelected}
      // The rail's action: open the round modal for the issue whose detail is
      // showing, and close the detail modal so only one dialog is up.
      onStartRound={() => {
        if (!selected) return
        setStartRoundFor(selected)
        setSelected(null)
      }}
    />
    {/* One owner of the modal's state: the card, the rail and the repair affordance
        all open this instance rather than each keeping their own. */}
    <StartRoundModal
      issueNumber={startRoundFor?.issue.number ?? null}
      issueTitle={startRoundFor?.issue.title}
      issueUrl={startRoundFor?.issue.html_url}
      repair={startRoundFor?.round_repair ?? null}
      // The branch the previous approval was reviewed on. `GapContinuity` dropped
      // `previous_branch`, and it needs no wire field: every Round declares a branch
      // (D5), so the last closed Round segment on the status this component already
      // holds *is* the answer. A render-time join, not derivation — the same pattern
      // A2/Q9 established for the card's branch compare.
      previousBranch={
        startRoundFor ? (lastClosedRound(startRoundFor.segments)?.branch ?? null) : null
      }
      onClose={() => setStartRoundFor(null)}
    />
    </>
  )
}
