import { useEffect, useMemo, useRef, useState } from 'react'
import {
  ActionIcon,
  Alert,
  Button,
  Combobox,
  Divider,
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
  const [generateLoading, setGenerateLoading] = useState(false)
  const [generateError, setGenerateError] = useState<string | null>(null)
  const [generateSuccess, setGenerateSuccess] = useState(false)
  const [addModalOpen, setAddModalOpen] = useState(false)

  const { data: repoData } = useRepoInfo()
  const { data: milestonesData } = useMilestones()
  const previewRequestId = useRef(0)
  // True once the user has manually edited the output path field
  const outputPathUserEdited = useRef(false)
  // Drives the reset button visibility (state so it triggers a re-render)
  const [outputPathIsCustom, setOutputPathIsCustom] = useState(false)

  // Fetch issue lists + statuses for all selected milestones (include closed issues for record)
  const { statuses, milestoneStatusByMilestone, isLoadingIssues, isLoadingStatuses } =
    useMilestoneIssues(selectedMilestones, true)

  // Keep a ref so the preview/generate effects can read the latest value without it being a dep
  const milestoneStatusRef = useRef(milestoneStatusByMilestone)
  milestoneStatusRef.current = milestoneStatusByMilestone

  // Count unapproved issues per milestone (warning indicator, does NOT exclude from downstream)
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

  // A milestone is excluded from downstream if it has any errors (list or status fetch failures)
  function hasErrors(n: number): boolean {
    const info = milestoneStatusRef.current[n]
    return !info || info.listFailed || info.statusErrorCount > 0
  }

  // Revert output path to the auto-generated default
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

  // Stable string of errored milestone numbers — lets output path effect react to error changes
  const erroredKey = selectedMilestones
    .filter((n) => milestoneStatusByMilestone[n]?.listFailed || (milestoneStatusByMilestone[n]?.statusErrorCount ?? 0) > 0)
    .sort((a, b) => a - b)
    .join(',')

  // Signature of the last preview request actually fired — used to skip redundant re-fires
  const lastPreviewSignatureRef = useRef('')

  // Auto-populate output path from repo + milestone names (excluding errored milestones)
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

  // Auto-preview: fires once all selected milestones' issue statuses have loaded,
  // but only if the effective request (included milestones + settings) actually changed.
  useEffect(() => {
    if (selectedMilestones.length === 0) {
      setPreviewKey(null)
      setPreviewError(null)
      setPreviewLoading(false)
      lastPreviewSignatureRef.current = ''
      return
    }

    // Still waiting for issue lists or status fetches — hold silently
    if (isLoadingIssues || isLoadingStatuses) {
      setPreviewError(null)
      return
    }

    // Exclude milestones with any errors
    const includedMilestones = selectedMilestones.filter((n) => !hasErrors(n))
    if (includedMilestones.length === 0) {
      setPreviewLoading(false)
      setPreviewError('All selected milestones failed to load — nothing to preview')
      lastPreviewSignatureRef.current = ''
      return
    }

    // Deduplicate: skip if the effective request is identical to what was last fired
    const qcIndex = contextItems.findIndex((i) => i.type === 'qc-record')
    const contextFiles = contextItems
      .filter((i): i is Extract<ContextItem, { type: 'file' }> => i.type === 'file')
      .map((item) => ({
        server_path: item.serverPath,
        position: (contextItems.indexOf(item) < qcIndex ? 'prepend' : 'append') as 'prepend' | 'append',
      }))
    const signature = JSON.stringify({ includedMilestones, tablesOnly, contextFiles })
    if (signature === lastPreviewSignatureRef.current) return
    lastPreviewSignatureRef.current = signature

    const id = ++previewRequestId.current
    setPreviewLoading(true)
    setPreviewError(null)

    previewRecord({ milestone_numbers: includedMilestones, tables_only: tablesOnly, output_path: '', context_files: contextFiles })
      .then((result) => {
        if (id !== previewRequestId.current) return
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
  }, [selectedMilestones, isLoadingIssues, isLoadingStatuses, tablesOnly, contextItems])

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

  return (
    <div style={{ display: 'flex', height: '100%', overflow: 'hidden' }}>
      {/* ── Left sidebar: all controls ───────────────────────────────────── */}
      <ResizableSidebar defaultWidth={400} minWidth={280} maxWidth={560}>
        <Stack gap="sm">

          {/* Milestones */}
          <MilestoneSelector
            selectedMilestones={selectedMilestones}
            onSelectedMilestonesChange={setSelectedMilestones}
            showOpenMilestones={showOpenMilestones}
            onShowOpenMilestonesChange={setShowOpenMilestones}
            statusByMilestone={milestoneStatusByMilestone}
            unapprovedByMilestone={unapprovedByMilestone}
          />

          <Divider />

          {/* Record Structure */}
          <div>
            <Text fw={600} size="sm" mb={4}>
              Record Structure
            </Text>
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

          <Divider />

          {/* Output path + Generate */}
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
      </ResizableSidebar>

      {/* ── Right pane: full-height PDF preview ──────────────────────────── */}
      <div style={{ flex: 1, position: 'relative', overflow: 'hidden' }}>
        {/* Keep the old iframe visible while a new preview is loading */}
        {previewKey && (
          <iframe
            key={previewKey}
            src={`/api/record/preview.pdf?key=${previewKey}`}
            style={{ width: '100%', height: '100%', border: 'none', display: 'block' }}
          />
        )}

        {/* Overlay: loading spinner */}
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

        {/* Overlay: error */}
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
              <Text size="sm">{previewError}</Text>
            </Alert>
          </div>
        )}

        {/* Placeholder: no milestones selected */}
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
      />
    </div>
  )
}

// ─── Milestone selector ───────────────────────────────────────────────────────

interface MilestoneSelectorProps {
  selectedMilestones: number[]
  onSelectedMilestonesChange: (v: number[]) => void
  showOpenMilestones: boolean
  onShowOpenMilestonesChange: (v: boolean) => void
  statusByMilestone: Record<number, MilestoneStatusInfo>
  unapprovedByMilestone: Record<number, number>
}

function MilestoneSelector({
  selectedMilestones,
  onSelectedMilestonesChange,
  showOpenMilestones,
  onShowOpenMilestonesChange,
  statusByMilestone,
  unapprovedByMilestone,
}: MilestoneSelectorProps) {
  const { data, isLoading, isError } = useMilestones()
  const [search, setSearch] = useState('')
  const combobox = useCombobox({ onDropdownClose: () => setSearch('') })

  const available = (data ?? []).filter(
    (m) => (showOpenMilestones || m.state === 'closed') && !selectedMilestones.includes(m.number)
  )
  const filtered = available.filter((m) =>
    m.title.toLowerCase().includes(search.toLowerCase())
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
      <Text fw={600} size="sm">Milestones</Text>

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
          <Combobox.Options>
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

      <Switch
        label="Show open milestones"
        size="xs"
        checked={showOpenMilestones}
        onChange={(e) => onShowOpenMilestonesChange(e.currentTarget.checked)}
      />

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
// Same as SelectedMilestoneCard in MilestoneFilter, but shows an unlock icon
// for open milestones instead of a closed pill for closed ones.

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
  // Any error (list or status) → red and excluded from downstream
  const isRed = statusInfo.listFailed || statusInfo.statusErrorCount > 0
  // Unapproved issues → yellow warning (still included in downstream)
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

          {/* Open milestone indicator */}
          {milestone.state !== 'closed' && (
            <Tooltip label="Milestone is not yet closed — record may be incomplete" withArrow>
              <IconLockOpen data-testid="open-milestone-indicator" size={14} color="#e67700" style={{ flexShrink: 0 }} />
            </Tooltip>
          )}

          {/* List fetch error */}
          {statusInfo.listFailed && statusInfo.listError && (
            <Tooltip label={`${statusInfo.listError} — excluded from record`} withArrow>
              <IconExclamationMark data-testid="list-error-indicator" size={14} color="#c92a2a" style={{ flexShrink: 0 }} />
            </Tooltip>
          )}

          {/* Status fetch errors — any count → red + excluded */}
          {statusInfo.statusErrorCount > 0 && errorLines && (
            <Tooltip label={errorLines} withArrow multiline>
              <span data-testid="status-error-count" style={{ color: '#c92a2a', display: 'flex', alignItems: 'center', gap: 2, flexShrink: 0 }}>
                <IconAlertCircle size={14} />
                {statusInfo.statusErrorCount}
              </span>
            </Tooltip>
          )}

          {/* Unapproved issues warning — included but flagged */}
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
          <>
            <style>{`
              @keyframes glisten {
                0%, 100% { opacity: 1; }
                50% { opacity: 0.35; }
              }
            `}</style>
            <Text size="xs" c="dimmed" style={{ animation: 'glisten 1.4s ease-in-out infinite' }}>
              {statusInfo.loadingCount} {statusInfo.loadingCount === 1 ? 'issue' : 'issues'} loading…
            </Text>
          </>
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
