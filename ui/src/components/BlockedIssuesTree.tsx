import { useEffect, useReducer, useRef } from 'react'
import { Anchor, Badge, Checkbox, Loader, Text, TextInput, Tooltip } from '@mantine/core'
import { IconChevronDown, IconChevronRight } from '@tabler/icons-react'
import type { BlockedIssueStatus, QCStatus } from '~/api/issues'
import { fetchBlockedIssues } from '~/api/issues'

// Swimlane header colors keyed by QC status (mirrors IssueDetailModal)
const STATUS_LANE_COLOR: Record<QCStatus['status'], string> = {
  approved:               '#dcfce7',
  changes_after_approval: '#dcfce7',
  awaiting_review:        '#dbeafe',
  approval_required:      '#dbeafe',
  change_requested:       '#fee2e2',
  in_progress:            '#fef9c3',
  changes_to_comment:     '#fef9c3',
}

interface TreeState {
  nodeData: Map<number, BlockedIssueStatus>
  // parent issue number → ordered child issue numbers
  childrenMap: Map<number, number[]>
  // issue number → parent issue number where it was first expanded
  expandedParent: Map<number, number>
  // visually collapsed nodes (expanded but hidden)
  collapsedSet: Set<number>
  loadingSet: Set<number>
  errorMap: Map<number, string>
}

type TreeAction =
  | { type: 'LOAD_START'; parentNumber: number }
  | { type: 'LOAD_SUCCESS'; parentNumber: number; children: BlockedIssueStatus[] }
  | { type: 'LOAD_ERROR'; parentNumber: number; error: string }
  | { type: 'EXPAND'; issueNumber: number; parentNumber: number }
  | { type: 'COLLAPSE'; issueNumber: number }
  | { type: 'UNCOLLAPSE'; issueNumber: number }

function treeReducer(state: TreeState, action: TreeAction): TreeState {
  switch (action.type) {
    case 'LOAD_START': {
      const loadingSet = new Set(state.loadingSet)
      loadingSet.add(action.parentNumber)
      return { ...state, loadingSet }
    }
    case 'LOAD_SUCCESS': {
      const loadingSet = new Set(state.loadingSet)
      loadingSet.delete(action.parentNumber)
      const nodeData = new Map(state.nodeData)
      const childNums: number[] = []
      for (const item of action.children) {
        nodeData.set(item.issue.number, item)
        childNums.push(item.issue.number)
      }
      const childrenMap = new Map(state.childrenMap)
      childrenMap.set(action.parentNumber, childNums)
      const errorMap = new Map(state.errorMap)
      errorMap.delete(action.parentNumber)
      return { ...state, loadingSet, nodeData, childrenMap, errorMap }
    }
    case 'LOAD_ERROR': {
      const loadingSet = new Set(state.loadingSet)
      loadingSet.delete(action.parentNumber)
      const errorMap = new Map(state.errorMap)
      errorMap.set(action.parentNumber, action.error)
      return { ...state, loadingSet, errorMap }
    }
    case 'EXPAND': {
      const expandedParent = new Map(state.expandedParent)
      expandedParent.set(action.issueNumber, action.parentNumber)
      // Un-collapse if it was previously collapsed
      const collapsedSet = new Set(state.collapsedSet)
      collapsedSet.delete(action.issueNumber)
      return { ...state, expandedParent, collapsedSet }
    }
    case 'COLLAPSE': {
      const collapsedSet = new Set(state.collapsedSet)
      collapsedSet.add(action.issueNumber)
      return { ...state, collapsedSet }
    }
    case 'UNCOLLAPSE': {
      const collapsedSet = new Set(state.collapsedSet)
      collapsedSet.delete(action.issueNumber)
      return { ...state, collapsedSet }
    }
    default:
      return state
  }
}

interface Props {
  rootIssueNumber: number
  onSelectionChange: (selections: Map<number, string>) => void
}

export function BlockedIssuesTree({ rootIssueNumber, onSelectionChange }: Props) {
  const [treeState, dispatch] = useReducer(treeReducer, {
    nodeData: new Map<number, BlockedIssueStatus>(),
    childrenMap: new Map<number, number[]>(),
    expandedParent: new Map<number, number>(),
    collapsedSet: new Set<number>(),
    loadingSet: new Set<number>(),
    errorMap: new Map<number, string>(),
  })

  const selectionsRef = useRef<Map<number, string>>(new Map())
  const [, forceUpdate] = useReducer((x: number) => x + 1, 0)

  function setSelections(updater: (prev: Map<number, string>) => Map<number, string>) {
    selectionsRef.current = updater(selectionsRef.current)
    forceUpdate()
    onSelectionChange(new Map(selectionsRef.current))
  }

  async function fetchChildren(parentNumber: number) {
    if (treeState.loadingSet.has(parentNumber)) return
    if (treeState.childrenMap.has(parentNumber)) return
    dispatch({ type: 'LOAD_START', parentNumber })
    try {
      const children = await fetchBlockedIssues(parentNumber)
      dispatch({ type: 'LOAD_SUCCESS', parentNumber, children })
    } catch (err) {
      dispatch({ type: 'LOAD_ERROR', parentNumber, error: (err as Error).message })
    }
  }

  async function expandNode(issueNumber: number, parentNumber: number) {
    if (treeState.expandedParent.has(issueNumber)) {
      // Already expanded at canonical location — just un-collapse if needed
      dispatch({ type: 'UNCOLLAPSE', issueNumber })
      return
    }
    dispatch({ type: 'EXPAND', issueNumber, parentNumber })
    await fetchChildren(issueNumber)
  }

  function toggleNode(issueNumber: number, parentNumber: number) {
    const { expandedParent, collapsedSet } = treeState
    const isExpanded = expandedParent.get(issueNumber) === parentNumber
    if (!isExpanded) {
      void expandNode(issueNumber, parentNumber)
    } else if (collapsedSet.has(issueNumber)) {
      dispatch({ type: 'UNCOLLAPSE', issueNumber })
    } else {
      dispatch({ type: 'COLLAPSE', issueNumber })
    }
  }

  useEffect(() => {
    void fetchChildren(rootIssueNumber)
  }, [rootIssueNumber]) // eslint-disable-line react-hooks/exhaustive-deps

  const rootChildren = treeState.childrenMap.get(rootIssueNumber) ?? []
  const isLoadingRoot = treeState.loadingSet.has(rootIssueNumber)
  const rootError = treeState.errorMap.get(rootIssueNumber)

  if (isLoadingRoot) {
    return (
      <div style={{ display: 'flex', alignItems: 'center', gap: 8, padding: '8px 0' }}>
        <Loader size="xs" />
        <Text size="sm" c="dimmed">Loading blocked issues...</Text>
      </div>
    )
  }

  if (rootError) {
    return <Text size="sm" c="red">Failed to load blocked issues: {rootError}</Text>
  }

  if (rootChildren.length === 0) {
    return <Text size="sm" c="dimmed">No issues are blocked by this issue.</Text>
  }

  return (
    <div>
      {rootChildren.map((num) => (
        <IssueNode
          key={num}
          issueNumber={num}
          parentNumber={rootIssueNumber}
          depth={0}
          treeState={treeState}
          selections={selectionsRef.current}
          setSelections={setSelections}
          toggleNode={toggleNode}
        />
      ))}
    </div>
  )
}

interface IssueNodeProps {
  issueNumber: number
  parentNumber: number
  depth: number
  treeState: TreeState
  selections: Map<number, string>
  setSelections: (updater: (prev: Map<number, string>) => Map<number, string>) => void
  toggleNode: (issueNumber: number, parentNumber: number) => void
}

function IssueNode({
  issueNumber,
  parentNumber,
  depth,
  treeState,
  selections,
  setSelections,
  toggleNode,
}: IssueNodeProps) {
  const { nodeData, childrenMap, expandedParent, collapsedSet, loadingSet, errorMap } = treeState

  const data = nodeData.get(issueNumber)
  if (!data) return null

  const { issue, qc_status } = data
  const isApproved = qc_status.status === 'approved'
  const isGrayed = !isApproved

  const canonicalParent = expandedParent.get(issueNumber)
  const isDuplicate = canonicalParent !== undefined && canonicalParent !== parentNumber

  const isExpanded = expandedParent.get(issueNumber) === parentNumber
  const isCollapsed = collapsedSet.has(issueNumber)
  const showChildren = isExpanded && !isCollapsed
  const isLoading = loadingSet.has(issueNumber)
  const nodeError = errorMap.get(issueNumber)
  const children = childrenMap.get(issueNumber) ?? []
  const isSelected = selections.has(issueNumber)
  const reason = selections.get(issueNumber) ?? ''

  const statusLabel = qc_status.status.replace(/_/g, ' ').replace(/\b\w/g, (c) => c.toUpperCase())
  const statusColor = STATUS_LANE_COLOR[qc_status.status]

  const indentPx = depth * 20

  function handleCheckboxChange(checked: boolean) {
    if (isGrayed || isDuplicate) return
    setSelections((prev) => {
      const next = new Map(prev)
      if (checked) next.set(issueNumber, reason)
      else next.delete(issueNumber)
      return next
    })
  }

  function handleReasonChange(text: string) {
    if (isGrayed || isDuplicate) return
    setSelections((prev) => {
      const next = new Map(prev)
      if (text || isSelected) next.set(issueNumber, text)
      else next.delete(issueNumber)
      return next
    })
  }

  // Chevron button — only shown for approved nodes (others can't have children)
  const chevron = isApproved && !isDuplicate ? (
    <span
      style={{ width: 16, display: 'flex', alignItems: 'center', justifyContent: 'center', flexShrink: 0, cursor: 'pointer' }}
      onClick={() => toggleNode(issueNumber, parentNumber)}
    >
      {isLoading ? (
        <Loader size={12} />
      ) : isExpanded && !isCollapsed ? (
        <IconChevronDown size={14} style={{ color: '#666' }} />
      ) : (
        <IconChevronRight size={14} style={{ color: '#bbb' }} />
      )}
    </span>
  ) : (
    <span style={{ width: 16, flexShrink: 0 }} />
  )

  const nodeContent = (
    <div style={{ marginLeft: indentPx, marginBottom: 4 }}>
      {/* Single row: chevron | checkbox | link | badge | textbox */}
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 6,
          opacity: isGrayed || isDuplicate ? 0.5 : 1,
        }}
      >
        {chevron}

        {!isGrayed && !isDuplicate ? (
          <Checkbox
            size="xs"
            checked={isSelected}
            onChange={(e) => handleCheckboxChange(e.currentTarget.checked)}
            style={{ flexShrink: 0 }}
          />
        ) : (
          <span style={{ width: 16, flexShrink: 0 }} />
        )}

        <Anchor
          href={issue.html_url}
          target="_blank"
          size="sm"
          style={{ flexShrink: 0, pointerEvents: isGrayed || isDuplicate ? 'none' : 'auto' }}
        >
          #{issue.number} {issue.title}
        </Anchor>

        <Badge
          size="xs"
          style={{
            backgroundColor: statusColor,
            color: '#333',
            border: '1px solid rgba(0,0,0,0.12)',
            textTransform: 'capitalize',
            flexShrink: 0,
          }}
        >
          {statusLabel}
        </Badge>

        {!isGrayed && !isDuplicate && (
          <TextInput
            size="xs"
            placeholder="Reason for unapproving..."
            value={reason}
            onChange={(e) => handleReasonChange(e.currentTarget.value)}
            style={{ flex: 1, minWidth: 0 }}
          />
        )}
      </div>

      {nodeError && (
        <Text size="xs" c="red" ml={38} mt={2}>
          Failed to load children: {nodeError}
        </Text>
      )}

      {showChildren && children.length > 0 && (
        <div style={{ marginTop: 2 }}>
          {children.map((childNum) => (
            <IssueNode
              key={childNum}
              issueNumber={childNum}
              parentNumber={issueNumber}
              depth={depth + 1}
              treeState={treeState}
              selections={selections}
              setSelections={setSelections}
              toggleNode={toggleNode}
            />
          ))}
        </div>
      )}
    </div>
  )

  if (isDuplicate) {
    const canonicalIssueTitle = nodeData.get(canonicalParent!)?.issue.title ?? ''
    const tooltipLabel = `Already expanded under issue #${canonicalParent}${canonicalIssueTitle ? ` — ${canonicalIssueTitle}` : ''}`
    return (
      <Tooltip label={tooltipLabel} withArrow position="right">
        <div>{nodeContent}</div>
      </Tooltip>
    )
  }

  return nodeContent
}
