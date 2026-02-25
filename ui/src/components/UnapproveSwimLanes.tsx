import { useEffect, useReducer, useState } from 'react'
import { createPortal } from 'react-dom'
import { ActionIcon, Anchor, Badge, Button, Loader, Modal, Stack, Text, TextInput, Tooltip } from '@mantine/core'
import type { DropResult } from '@hello-pangea/dnd'
import { DragDropContext, Draggable, Droppable } from '@hello-pangea/dnd'
import { IconMinus, IconPlus, IconX } from '@tabler/icons-react'
import { useQueryClient } from '@tanstack/react-query'
import type { BlockedIssueStatus, IssueStatusResponse, QCStatus } from '~/api/issues'
import { fetchSingleIssueStatus, postUnapprove } from '~/api/issues'

const STATUS_LANE_COLOR: Record<QCStatus['status'], string> = {
  approved:               '#dcfce7',
  changes_after_approval: '#dcfce7',
  awaiting_review:        '#dbeafe',
  approval_required:      '#dbeafe',
  change_requested:       '#fee2e2',
  in_progress:            '#fef9c3',
  changes_to_comment:     '#fef9c3',
}

function isApprovedStatus(s: QCStatus['status']): boolean {
  return s === 'approved' || s === 'changes_after_approval'
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

interface SwimState {
  nodeData:           Map<number, BlockedIssueStatus>
  loadedChildren:     Map<number, number[]>
  childrenVisible:    Set<number>
  loadingSet:         Set<number>
  errorMap:           Map<number, string>
  toUnapprove:        number[]
  reasons:            Map<number, string>
  blockedUnavailable: boolean
}

type SwimAction =
  | { type: 'INIT_ROOT'; root: BlockedIssueStatus }
  | { type: 'LOAD_START'; issueNumber: number }
  | { type: 'LOAD_SUCCESS'; issueNumber: number; children: BlockedIssueStatus[] }
  | { type: 'LOAD_ERROR'; issueNumber: number; error: string }
  | { type: 'BLOCKED_UNAVAILABLE'; issueNumber: number }
  | { type: 'EXPAND_CHILDREN'; issueNumber: number }
  | { type: 'COLLAPSE_CHILDREN'; issueNumber: number }
  | { type: 'ADD_TO_UNAPPROVE'; issueNumber: number }
  | { type: 'REMOVE_FROM_UNAPPROVE'; issueNumber: number }
  | { type: 'SET_REASON'; issueNumber: number; reason: string }

function swimReducer(state: SwimState, action: SwimAction): SwimState {
  switch (action.type) {
    case 'INIT_ROOT': {
      const nodeData = new Map(state.nodeData)
      nodeData.set(action.root.issue.number, action.root)
      const toUnapprove = isApprovedStatus(action.root.qc_status.status)
        ? [action.root.issue.number]
        : []
      return { ...state, nodeData, toUnapprove }
    }
    case 'LOAD_START': {
      const loadingSet = new Set(state.loadingSet)
      loadingSet.add(action.issueNumber)
      return { ...state, loadingSet }
    }
    case 'LOAD_SUCCESS': {
      const loadingSet = new Set(state.loadingSet)
      loadingSet.delete(action.issueNumber)
      const nodeData = new Map(state.nodeData)
      const childNums: number[] = []
      for (const item of action.children) {
        nodeData.set(item.issue.number, item)
        childNums.push(item.issue.number)
      }
      const loadedChildren = new Map(state.loadedChildren)
      loadedChildren.set(action.issueNumber, childNums)
      const errorMap = new Map(state.errorMap)
      errorMap.delete(action.issueNumber)
      return { ...state, loadingSet, nodeData, loadedChildren, errorMap }
    }
    case 'LOAD_ERROR': {
      const loadingSet = new Set(state.loadingSet)
      loadingSet.delete(action.issueNumber)
      const errorMap = new Map(state.errorMap)
      errorMap.set(action.issueNumber, action.error)
      return { ...state, loadingSet, errorMap }
    }
    case 'EXPAND_CHILDREN': {
      const childrenVisible = new Set(state.childrenVisible)
      childrenVisible.add(action.issueNumber)
      return { ...state, childrenVisible }
    }
    case 'COLLAPSE_CHILDREN': {
      const childrenVisible = new Set(state.childrenVisible)
      childrenVisible.delete(action.issueNumber)
      return { ...state, childrenVisible }
    }
    case 'ADD_TO_UNAPPROVE': {
      if (state.toUnapprove.includes(action.issueNumber)) return state
      return { ...state, toUnapprove: [...state.toUnapprove, action.issueNumber] }
    }
    case 'REMOVE_FROM_UNAPPROVE': {
      return { ...state, toUnapprove: state.toUnapprove.filter((n) => n !== action.issueNumber) }
    }
    case 'BLOCKED_UNAVAILABLE': {
      const loadingSet = new Set(state.loadingSet)
      loadingSet.delete(action.issueNumber)
      return { ...state, loadingSet, blockedUnavailable: true }
    }
    case 'SET_REASON': {
      const reasons = new Map(state.reasons)
      reasons.set(action.issueNumber, action.reason)
      return { ...state, reasons }
    }
    default:
      return state
  }
}

function initialState(): SwimState {
  return {
    nodeData:           new Map(),
    loadedChildren:     new Map(),
    childrenVisible:    new Set(),
    loadingSet:         new Set(),
    errorMap:           new Map(),
    toUnapprove:        [],
    reasons:            new Map(),
    blockedUnavailable: false,
  }
}

// ---------------------------------------------------------------------------
// Main component
// ---------------------------------------------------------------------------

interface Props {
  status: IssueStatusResponse
  onStatusUpdate: (status: IssueStatusResponse) => void
}

export function UnapproveSwimLanes({ status, onStatusUpdate }: Props) {
  const { issue } = status
  const [state, dispatch] = useReducer(swimReducer, undefined, initialState)
  const [fallbackReason, setFallbackReason] = useState('')
  const [postLoading, setPostLoading] = useState(false)
  const [postResultOpen, setPostResultOpen] = useState(false)
  const [postResults, setPostResults] = useState<Array<{ issueNumber: number; url: string; opened: boolean }>>([])
  const [postErrors, setPostErrors] = useState<Array<{ issueNumber: number; error: string }>>([])
  const queryClient = useQueryClient()

  useEffect(() => {
    dispatch({ type: 'INIT_ROOT', root: { issue, qc_status: status.qc_status } })
    dispatch({ type: 'EXPAND_CHILDREN', issueNumber: issue.number })
    void doFetch(issue.number)
  }, [issue.number]) // eslint-disable-line react-hooks/exhaustive-deps

  async function doFetch(issueNumber: number) {
    if (state.loadingSet.has(issueNumber)) return
    if (state.loadedChildren.has(issueNumber)) return
    dispatch({ type: 'LOAD_START', issueNumber })
    try {
      const res = await fetch(`/api/issues/${issueNumber}/blocked`)
      if (res.status === 501) {
        dispatch({ type: 'BLOCKED_UNAVAILABLE', issueNumber })
        return
      }
      if (!res.ok) {
        const data = await res.json().catch(() => null)
        dispatch({ type: 'LOAD_ERROR', issueNumber, error: data?.error ?? `Failed to fetch blocked issues: ${res.status}` })
        return
      }
      const children: BlockedIssueStatus[] = await res.json()
      dispatch({ type: 'LOAD_SUCCESS', issueNumber, children })
    } catch (err) {
      dispatch({ type: 'LOAD_ERROR', issueNumber, error: (err as Error).message })
    }
  }

  function expandChildren(issueNumber: number) {
    dispatch({ type: 'EXPAND_CHILDREN', issueNumber })
    void doFetch(issueNumber)
  }

  // Derived: allVisible = union of loadedChildren[n] for n in childrenVisible, minus toUnapprove
  const toUnapproveSet = new Set(state.toUnapprove)
  const allVisibleSet = new Set<number>()
  for (const parent of state.childrenVisible) {
    for (const n of state.loadedChildren.get(parent) ?? []) {
      if (!toUnapproveSet.has(n)) allVisibleSet.add(n)
    }
  }

  const impactedApprovals = [...allVisibleSet].filter((n) => {
    const d = state.nodeData.get(n)
    return d && isApprovedStatus(d.qc_status.status)
  })

  const notApproved = [...allVisibleSet].filter((n) => {
    const d = state.nodeData.get(n)
    return d && !isApprovedStatus(d.qc_status.status)
  })

  // Root issue: approved → already in toUnapprove; not approved → show in Not Approved lane
  const rootIsApproved = isApprovedStatus(status.qc_status.status)
  const rootData: BlockedIssueStatus = { issue: status.issue, qc_status: status.qc_status }

  function impactedBy(issueNum: number): number[] {
    return [...state.childrenVisible].filter((parent) =>
      (state.loadedChildren.get(parent) ?? []).includes(issueNum)
    )
  }

  const canPost = state.blockedUnavailable
    ? fallbackReason.trim() !== ''
    : state.toUnapprove.length > 0 && state.toUnapprove.every((n) => (state.reasons.get(n) ?? '').trim() !== '')

  async function handlePost() {
    setPostLoading(true)
    const results: typeof postResults = []
    const errors: typeof postErrors = []
    const toPost = state.blockedUnavailable
      ? [{ n: issue.number, reason: fallbackReason.trim() }]
      : state.toUnapprove.map((n) => ({ n, reason: (state.reasons.get(n) ?? '').trim() }))
    try {
      await Promise.all(
        toPost.map(async ({ n, reason }) => {
          try {
            const res = await postUnapprove(n, { reason })
            results.push({ issueNumber: n, url: res.unapproval_url, opened: res.opened })
          } catch (err) {
            errors.push({ issueNumber: n, error: (err as Error).message })
          }
        })
      )
      setPostResults(results)
      setPostErrors(errors)
      void queryClient.invalidateQueries({ queryKey: ['issue', 'status', issue.number] })
      const fresh = await fetchSingleIssueStatus(issue.number)
      onStatusUpdate(fresh)
    } finally {
      setPostLoading(false)
      setPostResultOpen(true)
    }
  }

  function onDragEnd(result: DropResult) {
    if (!result.destination) return
    const n = parseInt(result.draggableId, 10)
    const src = result.source.droppableId
    const dst = result.destination.droppableId
    if (src === 'impacted-approvals' && dst === 'to-unapprove') {
      dispatch({ type: 'ADD_TO_UNAPPROVE', issueNumber: n })
      dispatch({ type: 'EXPAND_CHILDREN', issueNumber: n })
      void doFetch(n)
    } else if (src === 'to-unapprove' && dst === 'impacted-approvals') {
      dispatch({ type: 'REMOVE_FROM_UNAPPROVE', issueNumber: n })
      dispatch({ type: 'COLLAPSE_CHILDREN', issueNumber: n })
    }
  }

  const isRootLoading = state.loadingSet.has(issue.number)
  const rootError = state.errorMap.get(issue.number)

  if (state.blockedUnavailable) {
    return (
      <>
        <div style={{ flex: 1, overflowY: 'auto', padding: '12px 0' }}>
          <Stack gap="xs" style={{ maxWidth: 380, margin: '0 auto' }}>
            <Text size="xs" c="dimmed">Unable to retrieve impacted issues — unapproving this issue only.</Text>
            <TextInput
              label="Reason (required)"
              placeholder="Required"
              value={fallbackReason}
              onChange={(e) => setFallbackReason(e.currentTarget.value)}
              styles={{ input: { borderColor: fallbackReason.trim() === '' ? '#e03131' : undefined } }}
            />
          </Stack>
        </div>
        <div style={{ borderTop: '1px solid var(--mantine-color-gray-3)', paddingTop: 12, paddingBottom: 12, display: 'flex', justifyContent: 'flex-end' }}>
          <Button color="red" loading={postLoading} disabled={!canPost} onClick={() => void handlePost()}>
            Unapprove
          </Button>
        </div>
        <ResultModal
          opened={postResultOpen}
          onClose={() => setPostResultOpen(false)}
          results={postResults}
          errors={postErrors}
          nodeData={state.nodeData}
        />
      </>
    )
  }

  return (
    <>
      <DragDropContext onDragEnd={onDragEnd}>
        <div style={{ flex: 1, overflowY: 'auto', padding: '12px 0' }}>
          <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr 1fr', gap: 12, alignItems: 'start', minHeight: 200 }}>

            {/* Lane 1: Not Approved */}
            <div>
              <LaneHeader color="#f1f3f5" title="Not Approved" count={notApproved.length + (rootIsApproved ? 0 : 1)} />
              <Stack gap="xs" p="xs" style={{ minHeight: 120 }}>
                {!rootIsApproved && (
                  <NotApprovedCard data={rootData} parents={[]} nodeData={state.nodeData} />
                )}
                {notApproved.length === 0 && rootIsApproved && !isRootLoading && (
                  <Text size="sm" c="dimmed" ta="center" py="sm">No unapproved issues</Text>
                )}
                {notApproved.map((n) => {
                  const data = state.nodeData.get(n)!
                  return (
                    <NotApprovedCard
                      key={n}
                      data={data}
                      parents={impactedBy(n)}
                      nodeData={state.nodeData}
                    />
                  )
                })}
              </Stack>
            </div>

            {/* Lane 2: Impacted Approvals */}
            <Droppable droppableId="impacted-approvals">
              {(provided) => (
                <div ref={provided.innerRef} {...provided.droppableProps}>
                  <LaneHeader color="#dcfce7" title="Impacted Approvals" count={impactedApprovals.length} />
                  <Stack gap="xs" p="xs" style={{ minHeight: 120 }}>
                    {isRootLoading && (
                      <div style={{ display: 'flex', alignItems: 'center', gap: 6, padding: '8px 0' }}>
                        <Loader size="xs" />
                        <Text size="xs" c="dimmed">Loading...</Text>
                      </div>
                    )}
                    {rootError && (
                      <Text size="xs" c="red">Error: {rootError}</Text>
                    )}
                    {impactedApprovals.length === 0 && !isRootLoading && !rootError && (
                      <Text size="sm" c="dimmed" ta="center" py="sm">No impacted approvals</Text>
                    )}
                    {impactedApprovals.map((n, idx) => {
                      const data = state.nodeData.get(n)!
                      const parents = impactedBy(n)
                      return (
                        <Draggable key={n} draggableId={String(n)} index={idx}>
                          {(dp, ds) => {
                            const el = (
                              <div
                                ref={dp.innerRef}
                                {...dp.draggableProps}
                                {...dp.dragHandleProps}
                                style={{ ...dp.draggableProps.style, opacity: ds.isDragging ? 0.85 : 1 }}
                              >
                                <ImpactedCard
                                data={data}
                                parents={parents}
                                nodeData={state.nodeData}
                                isExpanded={state.childrenVisible.has(n)}
                                isLoading={state.loadingSet.has(n)}
                                hasMultipleParents={parents.length > 1}
                                onExpand={() => expandChildren(n)}
                                onCollapse={() => dispatch({ type: 'COLLAPSE_CHILDREN', issueNumber: n })}
                              />
                              </div>
                            )
                            return ds.isDragging ? createPortal(el, document.body) : el
                          }}
                        </Draggable>
                      )
                    })}
                    {provided.placeholder}
                  </Stack>
                </div>
              )}
            </Droppable>

            {/* Lane 3: To Unapprove */}
            <Droppable droppableId="to-unapprove">
              {(provided, snapshot) => (
                <div
                  ref={provided.innerRef}
                  {...provided.droppableProps}
                  style={{
                    backgroundColor: snapshot.isDraggingOver ? 'rgba(224,49,49,0.08)' : undefined,
                    borderRadius: 6,
                    transition: 'background-color 0.15s',
                  }}
                >
                  <LaneHeader color="#fee2e2" title="To Unapprove" count={state.toUnapprove.length} />
                  <Stack gap="xs" p="xs" style={{ minHeight: 120 }}>
                    {state.toUnapprove.length === 0 && (
                      <Text size="sm" c="dimmed" ta="center" py="sm">Nothing to unapprove</Text>
                    )}
                    {state.toUnapprove.map((n, idx) => {
                      const data = state.nodeData.get(n)
                      if (!data) return null
                      return (
                        <Draggable key={n} draggableId={String(n)} index={idx} isDragDisabled={n === issue.number}>
                          {(dp, ds) => {
                            const el = (
                              <div ref={dp.innerRef} {...dp.draggableProps} {...dp.dragHandleProps}>
                                <ToUnapproveCard
                                  data={data}
                                  reason={state.reasons.get(n) ?? ''}
                                  isRoot={n === issue.number}
                                  onRemove={() => dispatch({ type: 'REMOVE_FROM_UNAPPROVE', issueNumber: n })}
                                  onReasonChange={(r) => dispatch({ type: 'SET_REASON', issueNumber: n, reason: r })}
                                />
                              </div>
                            )
                            return ds.isDragging ? createPortal(el, document.body) : el
                          }}
                        </Draggable>
                      )
                    })}
                    {provided.placeholder}
                  </Stack>
                </div>
              )}
            </Droppable>

          </div>
        </div>
      </DragDropContext>

      <div style={{ borderTop: '1px solid var(--mantine-color-gray-3)', paddingTop: 12, paddingBottom: 12, display: 'flex', justifyContent: 'flex-end', gap: 8 }}>
        <Button
          color="red"
          loading={postLoading}
          disabled={!canPost}
          onClick={() => void handlePost()}
        >
          Unapprove{state.toUnapprove.length > 1 ? ` (${state.toUnapprove.length})` : ''}
        </Button>
      </div>

      <ResultModal
        opened={postResultOpen}
        onClose={() => setPostResultOpen(false)}
        results={postResults}
        errors={postErrors}
        nodeData={state.nodeData}
      />
    </>
  )
}

// ---------------------------------------------------------------------------
// Result modal (shared between swim-lane and fallback modes)
// ---------------------------------------------------------------------------

function ResultModal({
  opened, onClose, results, errors, nodeData,
}: {
  opened: boolean
  onClose: () => void
  results: Array<{ issueNumber: number; url: string; opened: boolean }>
  errors: Array<{ issueNumber: number; error: string }>
  nodeData: Map<number, BlockedIssueStatus>
}) {
  const allFailed = errors.length > 0 && results.length === 0
  return (
    <Modal
      opened={opened}
      onClose={onClose}
      title={allFailed ? 'Unapprove Failed' : 'Unapproved'}
      size="sm"
      centered
      withinPortal={false}
    >
      <Stack gap="xs">
        {results.map((r) => {
          const title = nodeData.get(r.issueNumber)?.issue.title ?? `#${r.issueNumber}`
          return (
            <Text key={r.issueNumber} size="sm">
              <Anchor href={r.url} target="_blank">{title}</Anchor>
              {' '}{r.opened ? 'unapproved and reopened' : 'unapproved'}.
            </Text>
          )
        })}
        {errors.map((e) => (
          <Text key={e.issueNumber} c="red" size="sm">#{e.issueNumber}: {e.error}</Text>
        ))}
      </Stack>
    </Modal>
  )
}

// ---------------------------------------------------------------------------
// Lane header
// ---------------------------------------------------------------------------

function LaneHeader({ color, title, count }: { color: string; title: string; count: number }) {
  return (
    <div style={{
      backgroundColor: color,
      padding: '8px 12px',
      borderRadius: '6px 6px 0 0',
      display: 'flex',
      justifyContent: 'space-between',
      alignItems: 'center',
    }}>
      <Text size="sm" fw={600}>{title}</Text>
      <Badge size="sm" variant="light">{count}</Badge>
    </div>
  )
}

// ---------------------------------------------------------------------------
// ToUnapproveCard
// ---------------------------------------------------------------------------

function ToUnapproveCard({
  data,
  reason,
  isRoot,
  onRemove,
  onReasonChange,
}: {
  data: BlockedIssueStatus
  reason: string
  isRoot: boolean
  onRemove: () => void
  onReasonChange: (r: string) => void
}) {
  const { issue } = data
  return (
    <div style={{
      borderLeft: '3px solid #e03131',
      backgroundColor: 'white',
      borderRadius: '0 4px 4px 0',
      padding: '8px 10px',
      position: 'relative',
      boxShadow: '0 1px 2px rgba(0,0,0,0.06)',
    }}>
      {!isRoot && (
        <ActionIcon
          size="xs"
          variant="subtle"
          color="gray"
          style={{ position: 'absolute', top: 6, right: 6 }}
          onClick={onRemove}
          aria-label="Remove from unapprove"
        >
          <IconX size={12} />
        </ActionIcon>
      )}
      <Stack gap={4}>
        <Anchor
          href={issue.html_url}
          target="_blank"
          size="sm"
          fw={500}
          style={{ paddingRight: isRoot ? 0 : 20, wordBreak: 'break-word' }}
        >
          {issue.title}
        </Anchor>
        {issue.milestone && <Text size="xs" c="dimmed">{issue.milestone}</Text>}
        <TextInput
          size="xs"
          placeholder="Reason (required)"
          value={reason}
          onChange={(e) => onReasonChange(e.currentTarget.value)}
          styles={{ input: { borderColor: reason.trim() === '' ? '#e03131' : undefined } }}
        />
      </Stack>
    </div>
  )
}

// ---------------------------------------------------------------------------
// ImpactedCard (draggable)
// ---------------------------------------------------------------------------

function ImpactedCard({
  data,
  parents,
  nodeData,
  isExpanded,
  isLoading,
  hasMultipleParents,
  onExpand,
  onCollapse,
}: {
  data: BlockedIssueStatus
  parents: number[]
  nodeData: Map<number, BlockedIssueStatus>
  isExpanded: boolean
  isLoading: boolean
  hasMultipleParents: boolean
  onExpand: () => void
  onCollapse: () => void
}) {
  const { issue, qc_status } = data
  const statusLabel = qc_status.status.replace(/_/g, ' ')
  const statusColor = STATUS_LANE_COLOR[qc_status.status]
  const parentLabels = parents.map((p) => nodeData.get(p)?.issue.title ?? `#${p}`)

  return (
    <div style={{
      borderLeft: '3px solid #2f9e44',
      backgroundColor: 'white',
      borderRadius: '0 4px 4px 0',
      padding: '8px 10px',
      boxShadow: '0 1px 2px rgba(0,0,0,0.06)',
    }}>
      <Stack gap={4}>
        <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'flex-start', gap: 4 }}>
          <Anchor href={issue.html_url} target="_blank" size="sm" fw={500} style={{ flex: 1, minWidth: 0, wordBreak: 'break-word' }}>
            {issue.title}
          </Anchor>
          <div style={{ display: 'flex', gap: 2, flexShrink: 0 }}>
            {!isExpanded ? (
              <Tooltip label={isLoading ? 'Loading...' : 'Expand children'} withArrow>
                <ActionIcon
                  size="xs"
                  variant="subtle"
                  color="green"
                  onClick={onExpand}
                  disabled={isLoading}
                  aria-label="Expand children"
                >
                  {isLoading ? <Loader size={10} /> : <IconPlus size={12} />}
                </ActionIcon>
              </Tooltip>
            ) : (
              <Tooltip label={hasMultipleParents ? 'Shown by multiple parents' : 'Collapse children'} withArrow>
                <ActionIcon
                  size="xs"
                  variant="subtle"
                  color="green"
                  onClick={hasMultipleParents ? undefined : onCollapse}
                  disabled={hasMultipleParents}
                  aria-label="Collapse children"
                >
                  <IconMinus size={12} />
                </ActionIcon>
              </Tooltip>
            )}
          </div>
        </div>
        {issue.milestone && <Text size="xs" c="dimmed">{issue.milestone}</Text>}
        <Badge
          size="xs"
          style={{
            backgroundColor: statusColor,
            color: '#333',
            border: '1px solid rgba(0,0,0,0.12)',
            textTransform: 'capitalize',
            alignSelf: 'flex-start',
          }}
        >
          {statusLabel}
        </Badge>
        {parentLabels.length > 0 && (
          <Text size="xs" c="dimmed">Impacted by: {parentLabels.join(', ')}</Text>
        )}
      </Stack>
    </div>
  )
}

// ---------------------------------------------------------------------------
// NotApprovedCard
// ---------------------------------------------------------------------------

function NotApprovedCard({
  data,
  parents,
  nodeData,
}: {
  data: BlockedIssueStatus
  parents: number[]
  nodeData: Map<number, BlockedIssueStatus>
}) {
  const { issue, qc_status } = data
  const statusLabel = qc_status.status.replace(/_/g, ' ')
  const statusColor = STATUS_LANE_COLOR[qc_status.status]
  const parentLabels = parents.map((p) => nodeData.get(p)?.issue.title ?? `#${p}`)

  return (
    <div style={{
      borderLeft: '3px solid #adb5bd',
      backgroundColor: 'white',
      borderRadius: '0 4px 4px 0',
      padding: '8px 10px',
      opacity: 0.55,
      boxShadow: '0 1px 2px rgba(0,0,0,0.06)',
    }}>
      <Stack gap={4}>
        <Anchor href={issue.html_url} target="_blank" size="sm" fw={500} style={{ wordBreak: 'break-word' }}>
          {issue.title}
        </Anchor>
        {issue.milestone && <Text size="xs" c="dimmed">{issue.milestone}</Text>}
        <Badge
          size="xs"
          style={{
            backgroundColor: statusColor,
            color: '#333',
            border: '1px solid rgba(0,0,0,0.12)',
            textTransform: 'capitalize',
            alignSelf: 'flex-start',
          }}
        >
          {statusLabel}
        </Badge>
        {parentLabels.length > 0 && (
          <Text size="xs" c="dimmed">Impacted by: {parentLabels.join(', ')}</Text>
        )}
      </Stack>
    </div>
  )
}
