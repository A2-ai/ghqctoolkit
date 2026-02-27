import { useEffect, useMemo, useRef, useState } from 'react'
import {
  ActionIcon,
  Alert,
  Button,
  Combobox,
  InputBase,
  Loader,
  Stack,
  Switch,
  Text,
  TextInput,
  Tooltip,
  useCombobox,
} from '@mantine/core'
import {
  DragDropContext,
  Draggable,
  type DropResult,
  Droppable,
} from '@hello-pangea/dnd'
import {
  IconAlertCircle,
  IconAlertTriangle,
  IconArrowBackUp,
  IconChevronDown,
  IconChevronRight,
  IconExclamationMark,
  IconGripVertical,
  IconLock,
  IconLockOpen,
  IconPlus,
  IconX,
} from '@tabler/icons-react'
import { useMilestones } from '~/api/milestones'
import { type RecordRequest, generateRecord, previewRecord } from '~/api/record'
import { useRepoInfo } from '~/api/repo'
import { type MilestoneStatusInfo, useMilestoneIssues } from '~/api/issues'
import { ResizableSidebar } from './ResizableSidebar'
import { AddContextFileModal } from './AddContextFileModal'

// ─── Types ────────────────────────────────────────────────────────────────────

type ContextItem =
  | { id: string; type: 'file'; serverPath: string; displayName: string }
  | { id: 'qc-record'; type: 'qc-record' }

const MIN_RS_HEIGHT = 80
const COLLAPSED_HEIGHT = 36

// ─── RecordTab ────────────────────────────────────────────────────────────────

export function RecordTab() {
  const [selectedMilestones, setSelectedMilestones] = useState<number[]>([])
  const [showOpenMilestones, setShowOpenMilestones] = useState(false)
  const [tablesOnly, setTablesOnly] = useState(false)
  const [outputPath, setOutputPath] = useState('')
  const [contextItems, setContextItems] = useState<ContextItem[]>([
    { id: 'qc-record', type: 'qc-record' },
  ])
  const [previewKey, setPreviewKey] = useState<string | null>(null)
  const [previewLoading, setPreviewLoading] = useState(false)
  const [previewError, setPreviewError] = useState<string | null>(null)
  const [previewRetryCounter, setPreviewRetryCounter] = useState(0)
  const [generateLoading, setGenerateLoading] = useState(false)
  const [generateError, setGenerateError] = useState<string | null>(null)
  const [generateSuccess, setGenerateSuccess] = useState(false)
  const [addModalOpen, setAddModalOpen] = useState(false)
  const [fileTreeKey, setFileTreeKey] = useState(0)

  // Sidebar section layout state
  const [milestoneCollapsed, setMilestoneCollapsed] = useState(false)
  const [rsCollapsed, setRsCollapsed] = useState(false)
  const [rsHeight, setRsHeight] = useState(300)
  const [outputHeight, setOutputHeight] = useState<number | null>(null)
  const sidebarRef = useRef<HTMLDivElement>(null)
  const outputSectionRef = useRef<HTMLDivElement>(null)
  const minOutputHeightRef = useRef(0)
  const currentOutputHeightRef = useRef(0)
  const lastRsHeightRef = useRef(300)
  // Output drag refs
  const isDraggingOutput = useRef(false)
  const dragStartYOutput = useRef(0)
  const dragStartHeightOutput = useRef(0)
  // RS drag refs
  const isDraggingRs = useRef(false)
  const dragStartYRs = useRef(0)
  const dragStartHeightRs = useRef(0)
  // Keep ref in sync so drag closure always sees current value
  if (outputHeight !== null) currentOutputHeightRef.current = outputHeight

  const { data: repoData } = useRepoInfo()
  const { data: milestonesData } = useMilestones()
  const previewRequestId = useRef(0)
  const outputPathUserEdited = useRef(false)
  const [outputPathIsCustom, setOutputPathIsCustom] = useState(false)

  const { statuses, milestoneStatusByMilestone, isLoadingIssues, isLoadingStatuses } =
    useMilestoneIssues(selectedMilestones, true)

  const milestoneStatusRef = useRef(milestoneStatusByMilestone)
  milestoneStatusRef.current = milestoneStatusByMilestone

  const unapprovedByMilestone = useMemo(() => {
    const result: Record<number, number> = {}
    for (const n of selectedMilestones) {
      const milestoneName = (milestonesData ?? []).find((m) => m.number === n)?.title
      const milestoneStatuses = statuses.filter((s) => s.issue.milestone === milestoneName)
      result[n] = milestoneStatuses.filter(
        (s) =>
          s.qc_status.status !== 'approved' &&
          s.qc_status.status !== 'changes_after_approval',
      ).length
    }
    return result
  }, [selectedMilestones, statuses, milestonesData])

  function hasErrors(n: number): boolean {
    const info = milestoneStatusRef.current[n]
    return !info || info.listFailed || info.statusErrorCount > 0
  }

  function resetOutputPath() {
    outputPathUserEdited.current = false
    setOutputPathIsCustom(false)
    if (!repoData) { setOutputPath(''); return }
    const includedNumbers = selectedMilestones.filter((n) => !hasErrors(n))
    if (includedNumbers.length === 0) { setOutputPath(''); return }
    const names = includedNumbers
      .map((n) => (milestonesData ?? []).find((m) => m.number === n)?.title ?? String(n))
      .join('-')
      .replace(/\s+/g, '-')
    setOutputPath(`${repoData.repo}-${names}${tablesOnly ? '-tables' : ''}.pdf`)
  }

  const erroredKey = selectedMilestones
    .filter((n) => milestoneStatusByMilestone[n]?.listFailed || (milestoneStatusByMilestone[n]?.statusErrorCount ?? 0) > 0)
    .sort((a, b) => a - b)
    .join(',')

  const lastPreviewSignatureRef = useRef('')

  // Measure output section height on mount; set RS to 50% of sidebar
  useEffect(() => {
    if (sidebarRef.current && outputSectionRef.current) {
      const total = sidebarRef.current.clientHeight
      const outH = outputSectionRef.current.clientHeight
      if (total > 0 && outH > 0) {
        minOutputHeightRef.current = outH
        currentOutputHeightRef.current = outH
        setOutputHeight(outH)
        const half = Math.round(total * 0.5)
        setRsHeight(half)
        lastRsHeightRef.current = half
      }
    }
  }, [])

  // Unified drag-to-resize for output and RS handles
  useEffect(() => {
    const onMove = (e: MouseEvent) => {
      if (isDraggingOutput.current) {
        const delta = e.clientY - dragStartYOutput.current
        const min = minOutputHeightRef.current
        const max = (sidebarRef.current?.clientHeight ?? 600) - COLLAPSED_HEIGHT * 2
        setOutputHeight(Math.max(min, Math.min(max, dragStartHeightOutput.current + delta)))
      }
      if (isDraggingRs.current) {
        const delta = dragStartYRs.current - e.clientY
        const totalH = sidebarRef.current?.clientHeight ?? 600
        const maxH = totalH - currentOutputHeightRef.current - COLLAPSED_HEIGHT
        setRsHeight(Math.max(MIN_RS_HEIGHT, Math.min(maxH, dragStartHeightRs.current + delta)))
      }
    }
    const onUp = () => {
      isDraggingOutput.current = false
      isDraggingRs.current = false
      document.body.style.cursor = ''
      document.body.style.userSelect = ''
    }
    document.addEventListener('mousemove', onMove)
    document.addEventListener('mouseup', onUp)
    return () => {
      document.removeEventListener('mousemove', onMove)
      document.removeEventListener('mouseup', onUp)
    }
  }, [])

  const onOutputDragHandleMouseDown = (e: React.MouseEvent) => {
    isDraggingOutput.current = true
    dragStartYOutput.current = e.clientY
    dragStartHeightOutput.current = outputHeight ?? minOutputHeightRef.current
    document.body.style.cursor = 'row-resize'
    document.body.style.userSelect = 'none'
    e.preventDefault()
  }

  const onRsDragHandleMouseDown = (e: React.MouseEvent) => {
    isDraggingRs.current = true
    dragStartYRs.current = e.clientY
    dragStartHeightRs.current = rsHeight
    document.body.style.cursor = 'row-resize'
    document.body.style.userSelect = 'none'
    e.preventDefault()
  }

  function toggleRsCollapse() {
    if (rsCollapsed) {
      setRsHeight(lastRsHeightRef.current)
    } else {
      lastRsHeightRef.current = rsHeight
    }
    setRsCollapsed((c) => !c)
  }

  // Auto-populate output path
  useEffect(() => {
    if (outputPathUserEdited.current) return
    if (!repoData) return
    const includedNumbers = selectedMilestones.filter((n) => !hasErrors(n))
    if (includedNumbers.length === 0) {
      setOutputPath('')
      return
    }
    const milestoneNames = includedNumbers
      .map((n) => (milestonesData ?? []).find((m) => m.number === n)?.title ?? String(n))
      .join('-')
      .replace(/\s+/g, '-')
    setOutputPath(`${repoData.repo}-${milestoneNames}${tablesOnly ? '-tables' : ''}.pdf`)
  }, [selectedMilestones, milestonesData, repoData, tablesOnly, erroredKey])

  // Auto-preview
  useEffect(() => {
    if (selectedMilestones.length === 0) {
      setPreviewKey(null)
      setPreviewError(null)
      setPreviewLoading(false)
      lastPreviewSignatureRef.current = ''
      return
    }
    if (isLoadingIssues || isLoadingStatuses) {
      setPreviewError(null)
      return
    }
    const includedMilestones = selectedMilestones.filter((n) => !hasErrors(n))
    if (includedMilestones.length === 0) {
      setPreviewLoading(false)
      setPreviewError('All selected milestones failed to load — nothing to preview')
      lastPreviewSignatureRef.current = ''
      return
    }
    const qcIndex = contextItems.findIndex((i) => i.type === 'qc-record')
    const contextFiles = contextItems
      .filter((i): i is Extract<ContextItem, { type: 'file' }> => i.type === 'file')
      .map((item) => ({
        server_path: item.serverPath,
        position: (contextItems.indexOf(item) < qcIndex ? 'prepend' : 'append') as 'prepend' | 'append',
      }))
    const signature = JSON.stringify({ includedMilestones, tablesOnly, contextFiles })
    if (signature === lastPreviewSignatureRef.current) return

    const id = ++previewRequestId.current
    setPreviewLoading(true)
    setPreviewError(null)

    previewRecord({ milestone_numbers: includedMilestones, tables_only: tablesOnly, output_path: '', context_files: contextFiles })
      .then((result) => {
        if (id !== previewRequestId.current) return
        lastPreviewSignatureRef.current = signature
        setPreviewKey(result.key)
      })
      .catch((err: Error) => {
        if (id !== previewRequestId.current) return
        setPreviewError(err.message)
      })
      .finally(() => {
        if (id !== previewRequestId.current) return
        setPreviewLoading(false)
      })
  }, [selectedMilestones, isLoadingIssues, isLoadingStatuses, tablesOnly, contextItems, previewRetryCounter])

  async function handleGenerate() {
    setGenerateError(null)
    setGenerateSuccess(false)
    setGenerateLoading(true)
    try {
      const qcIndex = contextItems.findIndex((i) => i.type === 'qc-record')
      const req: RecordRequest = {
        milestone_numbers: selectedMilestones.filter((n) => !hasErrors(n)),
        tables_only: tablesOnly,
        output_path: outputPath,
        context_files: contextItems
          .filter((i): i is Extract<ContextItem, { type: 'file' }> => i.type === 'file')
          .map((item) => ({
            server_path: item.serverPath,
            position: contextItems.indexOf(item) < qcIndex ? 'prepend' : 'append',
          })),
      }
      await generateRecord(req)
      setGenerateSuccess(true)
      setFileTreeKey((k) => k + 1)
    } catch (err) {
      setGenerateError((err as Error).message)
    } finally {
      setGenerateLoading(false)
    }
  }

  function onDragEnd(result: DropResult) {
    if (!result.destination) return
    const src = result.source.index
    const dst = result.destination.index
    if (src === dst || contextItems[src].type === 'qc-record') return
    const items = [...contextItems]
    const [moved] = items.splice(src, 1)
    items.splice(dst, 0, moved)
    setContextItems(items)
  }

  function removeContextItem(id: string) {
    setContextItems((prev) => prev.filter((i) => i.id !== id))
  }

  function addContextItem(item: { serverPath: string; displayName: string }) {
    setContextItems((prev) => [
      ...prev,
      { id: `file-${Date.now()}`, type: 'file', serverPath: item.serverPath, displayName: item.displayName },
    ])
  }

  const bothCollapsed = milestoneCollapsed && rsCollapsed

  return (
    <div style={{ display: 'flex', height: '100%', overflow: 'hidden' }}>

      {/* ── Left sidebar ─────────────────────────────────────────────────── */}
      <ResizableSidebar defaultWidth={320} minWidth={280} maxWidth={560} noPadding>
        <div ref={sidebarRef} style={{ display: 'flex', flexDirection: 'column', height: '100%' }}>

          {/* ── Output Path + Generate ───────────────────────────────────── */}
          {/* flex:1 only when both below are collapsed; otherwise fixed to measured height */}
          <div
            ref={outputSectionRef}
            style={
              bothCollapsed
                ? { flex: 1, minHeight: 0, overflowY: 'auto', padding: 'var(--mantine-spacing-md)' }
                : { height: outputHeight ?? 'auto', flexShrink: 0, overflowY: 'auto', padding: 'var(--mantine-spacing-md)' }
            }
          >
            <Stack gap="sm">
              <TextInput
                label="Output Path"
                placeholder="/path/to/report.pdf"
                size="xs"
                value={outputPath}
                onChange={(e) => {
                  const val = e.currentTarget.value
                  outputPathUserEdited.current = val !== ''
                  setOutputPathIsCustom(val !== '')
                  setOutputPath(val)
                }}
                rightSection={outputPathIsCustom && selectedMilestones.length > 0 ? (
                  <Tooltip label="Reset to default" withArrow position="top">
                    <ActionIcon
                      size="xs"
                      variant="transparent"
                      color="gray"
                      onClick={resetOutputPath}
                      aria-label="Reset output path to default"
                    >
                      <IconArrowBackUp size={13} />
                    </ActionIcon>
                  </Tooltip>
                ) : undefined}
              />
              {generateError && (
                <Alert color="red" p="xs">
                  <Text size="xs">{generateError}</Text>
                </Alert>
              )}
              {generateSuccess && (
                <Alert color="green" p="xs">
                  <Text size="xs">PDF written to {outputPath}</Text>
                </Alert>
              )}
              <Button
                fullWidth
                size="sm"
                color="green"
                onClick={handleGenerate}
                loading={generateLoading}
                disabled={selectedMilestones.length === 0 || !outputPath.trim()}
              >
                Generate
              </Button>
            </Stack>
          </div>

          {/* ── Drag handle A: between Output and Milestones ─────────────── */}
          {/* Hidden when both sections below are collapsed */}
          {!bothCollapsed && outputHeight !== null && (
            <div
              onMouseDown={onOutputDragHandleMouseDown}
              style={{
                height: 6,
                flexShrink: 0,
                cursor: 'row-resize',
                borderTop: '1px solid var(--mantine-color-gray-3)',
              }}
            />
          )}

          {/* ── Milestones (collapsible) ─────────────────────────────────── */}
          <div style={
            milestoneCollapsed
              ? {
                  height: COLLAPSED_HEIGHT,
                  flexShrink: 0,
                  // needs its own top border when drag handle A is hidden
                  borderTop: bothCollapsed ? '1px solid var(--mantine-color-gray-3)' : undefined,
                }
              : { flex: 1, minHeight: 0, display: 'flex', flexDirection: 'column' }
          }>
            <div
              style={{
                display: 'flex',
                alignItems: 'center',
                gap: 4,
                padding: '0 var(--mantine-spacing-md)',
                height: COLLAPSED_HEIGHT,
                flexShrink: 0,
                borderBottom: milestoneCollapsed ? undefined : '1px solid var(--mantine-color-gray-3)',
                cursor: 'pointer',
              }}
              onClick={() => setMilestoneCollapsed((c) => !c)}
              title={milestoneCollapsed ? 'Expand' : 'Collapse'}
            >
              <ActionIcon size="xs" variant="subtle" tabIndex={-1} style={{ pointerEvents: 'none' }}>
                {milestoneCollapsed ? <IconChevronRight size={14} /> : <IconChevronDown size={14} />}
              </ActionIcon>
              <Text fw={600} size="sm">Milestones</Text>
            </div>
            {!milestoneCollapsed && (
              <div style={{ flex: 1, overflowY: 'auto', padding: 'var(--mantine-spacing-md)' }}>
                <Stack gap="sm">
                  <Switch
                    label="Include open milestones"
                    size="xs"
                    checked={showOpenMilestones}
                    onChange={(e) => setShowOpenMilestones(e.currentTarget.checked)}
                  />
                  <MilestoneCombobox
                    selectedMilestones={selectedMilestones}
                    onSelectedMilestonesChange={setSelectedMilestones}
                    showOpenMilestones={showOpenMilestones}
                    statusByMilestone={milestoneStatusByMilestone}
                    unapprovedByMilestone={unapprovedByMilestone}
                  />
                </Stack>
              </div>
            )}
          </div>

          {/* ── Record Structure (resizable + collapsible) ───────────────── */}
          {/* flex:1 (fills space) when milestones is collapsed; fixed height otherwise */}
          <div style={
            rsCollapsed
              ? { height: COLLAPSED_HEIGHT, flexShrink: 0 }
              : milestoneCollapsed
                ? { flex: 1, minHeight: 0, display: 'flex', flexDirection: 'column' }
                : { height: rsHeight, flexShrink: 0, display: 'flex', flexDirection: 'column' }
          }>
            {/* RS drag handle — only when both milestones and RS are expanded */}
            {!milestoneCollapsed && !rsCollapsed && (
              <div
                onMouseDown={onRsDragHandleMouseDown}
                style={{
                  height: 6,
                  flexShrink: 0,
                  cursor: 'row-resize',
                  borderTop: '1px solid var(--mantine-color-gray-3)',
                }}
              />
            )}

            {/* Header — top border when RS drag handle is absent */}
            <div
              style={{
                display: 'flex',
                alignItems: 'center',
                gap: 4,
                padding: '0 var(--mantine-spacing-md)',
                height: COLLAPSED_HEIGHT,
                flexShrink: 0,
                borderTop: (rsCollapsed || milestoneCollapsed)
                  ? '1px solid var(--mantine-color-gray-3)'
                  : undefined,
                cursor: 'pointer',
              }}
              onClick={toggleRsCollapse}
              title={rsCollapsed ? 'Expand' : 'Collapse'}
            >
              <ActionIcon size="xs" variant="subtle" tabIndex={-1} style={{ pointerEvents: 'none' }}>
                {rsCollapsed ? <IconChevronRight size={14} /> : <IconChevronDown size={14} />}
              </ActionIcon>
              <Text fw={600} size="sm">Record Structure</Text>
            </div>

            {!rsCollapsed && (
              <div style={{ flex: 1, overflowY: 'auto', padding: '0 var(--mantine-spacing-md) var(--mantine-spacing-md)' }}>
                <Text size="xs" c="dimmed" mb={8}>
                  Add documents to provide context to the QC findings
                </Text>
                <DragDropContext onDragEnd={onDragEnd}>
                  <Droppable droppableId="context-list">
                    {(provided) => (
                      <div
                        ref={provided.innerRef}
                        {...provided.droppableProps}
                        style={{
                          border: '1px solid var(--mantine-color-gray-3)',
                          borderRadius: 6,
                          overflow: 'hidden',
                        }}
                      >
                        {contextItems.map((item, index) => (
                          <Draggable
                            key={item.id}
                            draggableId={item.id}
                            index={index}
                            isDragDisabled={item.type === 'qc-record'}
                          >
                            {(provided, snapshot) =>
                              item.type === 'qc-record' ? (
                                <div
                                  ref={provided.innerRef}
                                  {...provided.draggableProps}
                                  style={{
                                    display: 'flex',
                                    alignItems: 'center',
                                    gap: 6,
                                    padding: '7px 10px',
                                    backgroundColor: '#d7e7d3',
                                    borderTop: index > 0 ? '1px solid var(--mantine-color-gray-3)' : undefined,
                                    borderBottom: index < contextItems.length - 1 ? '1px solid var(--mantine-color-gray-3)' : undefined,
                                    ...provided.draggableProps.style,
                                  }}
                                >
                                  <IconLock size={12} color="#2f7a3b" style={{ flexShrink: 0 }} />
                                  <Text size="xs" fw={600} c="#2f7a3b" style={{ flex: 1 }}>
                                    QC Record
                                  </Text>
                                  <Tooltip
                                    label={tablesOnly
                                      ? 'Tables only: include only the QC summary tables'
                                      : 'Full record: include QC history and summary tables'}
                                    withArrow
                                    position="top"
                                  >
                                    <Switch
                                      label="Tables only"
                                      size="xs"
                                      color="#2f7a3b"
                                      checked={tablesOnly}
                                      onChange={(e) => setTablesOnly(e.currentTarget.checked)}
                                      styles={{
                                        label: { color: tablesOnly ? '#1a1a1a' : '#868e96', fontSize: 11, paddingLeft: 6, fontWeight: 700 },
                                        track: {
                                          borderColor: tablesOnly ? '#2f7a3b' : '#adb5bd',
                                          backgroundColor: tablesOnly ? undefined : '#e9ecef',
                                        },
                                        thumb: { borderColor: tablesOnly ? '#2f7a3b' : '#adb5bd' },
                                      }}
                                    />
                                  </Tooltip>
                                </div>
                              ) : (
                                <div
                                  ref={provided.innerRef}
                                  {...provided.draggableProps}
                                  style={{
                                    display: 'flex',
                                    alignItems: 'center',
                                    gap: 6,
                                    padding: '6px 10px',
                                    backgroundColor: snapshot.isDragging ? '#f8f9fa' : 'white',
                                    borderTop: index > 0 ? '1px solid var(--mantine-color-gray-3)' : undefined,
                                    ...provided.draggableProps.style,
                                  }}
                                >
                                  <div {...provided.dragHandleProps} style={{ flexShrink: 0, cursor: 'grab', display: 'flex' }}>
                                    <IconGripVertical size={14} color="var(--mantine-color-gray-5)" />
                                  </div>
                                  <Text
                                    size="xs"
                                    style={{ flex: 1, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}
                                  >
                                    {item.displayName}
                                  </Text>
                                  <ActionIcon
                                    size="xs"
                                    variant="transparent"
                                    color="dark"
                                    onClick={() => removeContextItem(item.id)}
                                    aria-label={`Remove ${item.displayName}`}
                                    style={{ flexShrink: 0 }}
                                  >
                                    <IconX size={11} />
                                  </ActionIcon>
                                </div>
                              )
                            }
                          </Draggable>
                        ))}
                        {provided.placeholder}
                      </div>
                    )}
                  </Droppable>
                </DragDropContext>
                <Button
                  leftSection={<IconPlus size={12} />}
                  variant="light"
                  size="xs"
                  mt={6}
                  onClick={() => setAddModalOpen(true)}
                >
                  Add File
                </Button>
              </div>
            )}
          </div>

        </div>
      </ResizableSidebar>

      {/* ── Right pane: full-height PDF preview ──────────────────────────── */}
      <div style={{ flex: 1, position: 'relative', overflow: 'hidden' }}>
        {previewKey && (
          <iframe
            key={previewKey}
            src={`/api/record/preview.pdf?key=${previewKey}`}
            style={{ width: '100%', height: '100%', border: 'none', display: 'block' }}
          />
        )}
        {previewLoading && (
          <div style={{
            position: 'absolute',
            inset: 0,
            backgroundColor: previewKey ? 'rgba(255,255,255,0.65)' : 'white',
            display: 'flex',
            flexDirection: 'column',
            alignItems: 'center',
            justifyContent: 'center',
            gap: 12,
          }}>
            <Loader size="md" />
            <Text size="sm" c="dimmed">Generating preview…</Text>
          </div>
        )}
        {previewError && !previewLoading && (
          <div style={{
            position: 'absolute',
            inset: 0,
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
            padding: 32,
          }}>
            <Alert color="red" style={{ maxWidth: 480 }}>
              <Text size="sm" mb={8}>{previewError}</Text>
              <Button
                size="xs"
                color="red"
                variant="light"
                onClick={() => setPreviewRetryCounter((c) => c + 1)}
              >
                Retry
              </Button>
            </Alert>
          </div>
        )}
        {!previewKey && !previewLoading && !previewError && (
          <div style={{
            position: 'absolute',
            inset: 0,
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
          }}>
            <Text c="dimmed" size="sm">Select a milestone to generate a preview</Text>
          </div>
        )}
      </div>

      <AddContextFileModal
        opened={addModalOpen}
        onClose={() => setAddModalOpen(false)}
        onAdd={addContextItem}
        fileTreeKey={fileTreeKey}
      />
    </div>
  )
}

// ─── MilestoneCombobox ────────────────────────────────────────────────────────

interface MilestoneComboboxProps {
  selectedMilestones: number[]
  onSelectedMilestonesChange: (v: number[]) => void
  showOpenMilestones: boolean
  statusByMilestone: Record<number, MilestoneStatusInfo>
  unapprovedByMilestone: Record<number, number>
}

function MilestoneCombobox({
  selectedMilestones,
  onSelectedMilestonesChange,
  showOpenMilestones,
  statusByMilestone,
  unapprovedByMilestone,
}: MilestoneComboboxProps) {
  const { data, isLoading, isError } = useMilestones()
  const [search, setSearch] = useState('')
  const combobox = useCombobox({ onDropdownClose: () => setSearch('') })

  const available = (data ?? []).filter(
    (m) => (showOpenMilestones || m.state === 'closed') && !selectedMilestones.includes(m.number),
  )
  const filtered = available.filter((m) =>
    m.title.toLowerCase().includes(search.toLowerCase()),
  )
  const selectedItems = (data ?? []).filter((m) => selectedMilestones.includes(m.number))

  function add(number: number) {
    onSelectedMilestonesChange([...selectedMilestones, number])
    combobox.closeDropdown()
    setSearch('')
  }

  function remove(number: number) {
    onSelectedMilestonesChange(selectedMilestones.filter((n) => n !== number))
  }

  return (
    <Stack gap="sm">
      <Combobox store={combobox} onOptionSubmit={(val) => add(Number(val))}>
        <Combobox.Target>
          <InputBase
            placeholder="Search milestones…"
            size="xs"
            value={search}
            rightSection={isLoading ? <Loader size={12} /> : <Combobox.Chevron />}
            onChange={(e) => { setSearch(e.currentTarget.value); combobox.openDropdown() }}
            onClick={() => combobox.openDropdown()}
            onFocus={() => combobox.openDropdown()}
          />
        </Combobox.Target>
        <Combobox.Dropdown>
          <Combobox.Options style={{ maxHeight: 360, overflowY: 'auto' }}>
            {isError && <Combobox.Empty>Failed to load</Combobox.Empty>}
            {!isLoading && !isError && filtered.length === 0 && (
              <Combobox.Empty>No milestones found</Combobox.Empty>
            )}
            {[...filtered].reverse().map((m) => (
              <Combobox.Option key={m.number} value={String(m.number)}>
                <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                  <Text size="sm">{m.title}</Text>
                  {m.state !== 'closed' && (
                    <Tooltip label="This milestone is not closed" withArrow>
                      <IconAlertTriangle size={13} color="#f59f00" style={{ flexShrink: 0 }} />
                    </Tooltip>
                  )}
                </div>
                <Text size="xs" c="dimmed">
                  {m.open_issues} open · {m.closed_issues} closed
                </Text>
              </Combobox.Option>
            ))}
          </Combobox.Options>
        </Combobox.Dropdown>
      </Combobox>

      {selectedItems.length > 0 && (
        <Stack gap={4}>
          {selectedItems.map((m) => (
            <RecordMilestoneCard
              key={m.number}
              milestone={m}
              statusInfo={statusByMilestone[m.number] ?? { listFailed: false, listError: null, loadingCount: 0, statusErrorCount: 0, statusErrors: [], statusAttemptedCount: 0 }}
              unapprovedCount={unapprovedByMilestone[m.number] ?? 0}
              onRemove={() => remove(m.number)}
            />
          ))}
        </Stack>
      )}
    </Stack>
  )
}

// ─── RecordMilestoneCard ──────────────────────────────────────────────────────

function RecordMilestoneCard({
  milestone,
  statusInfo,
  unapprovedCount,
  onRemove,
}: {
  milestone: import('~/api/milestones').Milestone
  statusInfo: MilestoneStatusInfo
  unapprovedCount: number
  onRemove: () => void
}) {
  const isRed = statusInfo.listFailed || statusInfo.statusErrorCount > 0
  const isYellow = !isRed && unapprovedCount > 0

  const bgColor = isRed ? '#ffe3e3' : isYellow ? '#fff3bf' : '#d7e7d3'
  const borderColor = isRed ? '#ff8787' : isYellow ? '#fcc419' : '#aacca6'

  const errorLines =
    statusInfo.statusErrors.length > 0 ? (
      <div>
        {statusInfo.statusErrors.map((e) => (
          <div key={e.issue_number}>#{e.issue_number}: {e.error}</div>
        ))}
      </div>
    ) : null

  return (
    <div style={{
      display: 'flex',
      alignItems: 'center',
      justifyContent: 'space-between',
      gap: 6,
      padding: '6px 8px',
      borderRadius: 6,
      backgroundColor: bgColor,
      border: `1px solid ${borderColor}`,
    }}>
      <div style={{ minWidth: 0 }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 6, minWidth: 0 }}>
          <Text size="sm" fw={600} truncate="end">{milestone.title}</Text>
          {milestone.state !== 'closed' && (
            <Tooltip label="Milestone is not yet closed — record may be incomplete" withArrow>
              <IconLockOpen data-testid="open-milestone-indicator" size={14} color="#e67700" style={{ flexShrink: 0 }} />
            </Tooltip>
          )}
          {statusInfo.listFailed && statusInfo.listError && (
            <Tooltip label={`${statusInfo.listError} — excluded from record`} withArrow>
              <IconExclamationMark data-testid="list-error-indicator" size={14} color="#c92a2a" style={{ flexShrink: 0 }} />
            </Tooltip>
          )}
          {statusInfo.statusErrorCount > 0 && errorLines && (
            <Tooltip label={errorLines} withArrow multiline>
              <span data-testid="status-error-count" style={{ color: '#c92a2a', display: 'flex', alignItems: 'center', gap: 2, flexShrink: 0 }}>
                <IconAlertCircle size={14} />
                {statusInfo.statusErrorCount}
              </span>
            </Tooltip>
          )}
          {isYellow && (
            <Tooltip
              label={`${unapprovedCount} issue${unapprovedCount !== 1 ? 's' : ''} not yet approved`}
              withArrow
            >
              <span data-testid="unapproved-warning" style={{ color: '#e67700', display: 'flex', alignItems: 'center', gap: 2, flexShrink: 0 }}>
                <IconAlertTriangle size={14} />
                {unapprovedCount}
              </span>
            </Tooltip>
          )}
        </div>
        <Text size="xs" c="dimmed">
          {milestone.open_issues} open · {milestone.closed_issues} closed
        </Text>
        {statusInfo.loadingCount > 0 && (
          <Text size="xs" c="dimmed" style={{ animation: 'glisten 1.4s ease-in-out infinite' }}>
            {statusInfo.loadingCount} {statusInfo.loadingCount === 1 ? 'issue' : 'issues'} loading…
          </Text>
        )}
      </div>
      <ActionIcon
        size="xs"
        variant="transparent"
        color="dark"
        onClick={onRemove}
        style={{ flexShrink: 0 }}
        aria-label={`Remove ${milestone.title}`}
      >
        <IconX size={12} />
      </ActionIcon>
    </div>
  )
}
