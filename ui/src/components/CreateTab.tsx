import { useState } from 'react'
import { Button, Select, Text, TextInput, Textarea, Stack, Loader, Tooltip } from '@mantine/core'
import { useQueryClient } from '@tanstack/react-query'
import { useMilestones } from '~/api/milestones'
import { useIssuesForMilestone } from '~/api/issues'
import { useRepoInfo } from '~/api/repo'
import { postCreateMilestone, postCreateIssues, toCreateIssueRequest } from '~/api/create'
import { ResizableSidebar } from './ResizableSidebar'
import { ExistingIssueCard } from './ExistingIssueCard'
import { AddFileCard } from './AddFileCard'
import { CreateIssueModal } from './CreateIssueModal'
import type { QueuedItem } from './CreateIssueModal'
import { QueuedIssueCard } from './QueuedIssueCard'
import { CreateResultModal } from './CreateResultModal'
import type { CreateOutcome } from './CreateResultModal'
import { BatchRelevantFilesModal } from './BatchRelevantFilesModal'
import type { RelevantFileDraft } from './CreateIssueModal'

type MilestoneMode = 'select' | 'new'

export function CreateTab() {
  const [mode, setMode] = useState<MilestoneMode>('select')
  const [selectedMilestone, setSelectedMilestone] = useState<number | null>(null)
  const [newName, setNewName] = useState('')
  const [newDesc, setNewDesc] = useState('')
  const [modalOpen, setModalOpen] = useState(false)
  const [editingIndex, setEditingIndex] = useState<number | null>(null)
  const [queuedItems, setQueuedItems] = useState<QueuedItem[]>([])
  const [isCreating, setIsCreating] = useState(false)
  const [createOutcome, setCreateOutcome] = useState<CreateOutcome | null>(null)
  const [resultOpen, setResultOpen] = useState(false)
  const [batchOpen, setBatchOpen] = useState(false)
  const queryClient = useQueryClient()
  const { data: repoInfo } = useRepoInfo()
  const dirtyFiles = new Set(repoInfo?.dirty_files ?? [])
  const gitStatus = repoInfo?.git_status ?? 'clean'
  const createWarning: { color: string; tooltip: string } | null = (() => {
    if (gitStatus === 'diverged') return { color: 'red',    tooltip: 'Resolve divergence before creating issues' }
    if (gitStatus === 'ahead')    return { color: 'orange', tooltip: 'Push to synchronize with remote before creating issues' }
    if (gitStatus === 'behind')   return { color: 'orange', tooltip: 'Pull to synchronize with remote before creating issues' }
    if (queuedItems.some(item => dirtyFiles.has(item.file)))
      return { color: 'yellow', tooltip: 'Recommended to be in a clean git state before creating issues' }
    return null
  })()
  const { data: milestones = [], isLoading } = useMilestones()
  const { data: milestoneIssues = [], isLoading: issuesLoading } =
    useIssuesForMilestone(mode === 'select' ? selectedMilestone : null)

  const openMilestones = milestones.filter(m => m.state === 'open')
  const nameConflict = newName.trim().length > 0
    && milestones.some(m => m.title.toLowerCase() === newName.trim().toLowerCase())
  const milestoneTitle = mode === 'select'
    ? (milestones.find(m => m.number === selectedMilestone)?.title ?? null)
    : (newName.trim() || null)

  // Conflict detection
  const existingFiles = new Set(mode === 'select' ? milestoneIssues.map(i => i.title) : [])
  const queuedFileCounts = queuedItems.reduce<Map<string, number>>((acc, item) => {
    acc.set(item.file, (acc.get(item.file) ?? 0) + 1)
    return acc
  }, new Map())

  function getConflictReason(item: QueuedItem): string | undefined {
    if (existingFiles.has(item.file))
      return `"${item.file}" already has an issue in milestone "${milestoneTitle}"`
    if ((queuedFileCounts.get(item.file) ?? 0) > 1)
      return `"${item.file}" is queued more than once`
    return undefined
  }

  const milestoneReady = (mode === 'select' && selectedMilestone !== null) ||
    (mode === 'new' && newName.trim().length > 0 && !nameConflict)
  const hasConflicts = queuedItems.some(item => getConflictReason(item) !== undefined)
  const canCreate = milestoneReady && !hasConflicts && queuedItems.length > 0

  function createBlockReason(): string | undefined {
    if (!milestoneReady) return 'Select or name a milestone first'
    if (hasConflicts) return 'Resolve all conflicts before creating'
    if (queuedItems.length === 0) return 'Queue at least one issue'
    return undefined
  }
  const blockReason = createBlockReason()

  function handleBatchApply(indices: number[], draft: RelevantFileDraft) {
    setQueuedItems(prev => prev.map((item, i) => {
      if (!indices.includes(i)) return item
      if (item.relevantFiles.some(rf => rf.file === draft.file)) return item
      return { ...item, relevantFiles: [...item.relevantFiles, draft] }
    }))
  }

  async function handleCreate() {
    setIsCreating(true)
    try {
      let milestoneNumber: number
      let resolvedTitle: string
      let milestoneCreated = false
      if (mode === 'select') {
        milestoneNumber = selectedMilestone!
        resolvedTitle = milestoneTitle ?? ''
      } else {
        const ms = await postCreateMilestone(newName.trim(), newDesc.trim() || null)
        milestoneNumber = ms.number
        resolvedTitle = ms.title
        milestoneCreated = true
        queryClient.invalidateQueries({ queryKey: ['milestones'] })
      }

      const batchFiles = new Set(queuedItems.map((q) => q.file))
      const requests = queuedItems.map((item) => toCreateIssueRequest(item, batchFiles))
      const responses = await postCreateIssues(milestoneNumber, requests)

      queryClient.invalidateQueries({ queryKey: ['milestones', milestoneNumber, 'issues'] })

      setCreateOutcome({
        ok: true,
        milestoneNumber,
        milestoneTitle: resolvedTitle,
        milestoneCreated,
        created: responses.map((r, i) => ({
          file: queuedItems[i].file,
          issueUrl: r.issue_url,
          issueNumber: r.issue_number,
        })),
      })
    } catch (err) {
      setCreateOutcome({ ok: false, error: (err as Error).message })
    } finally {
      setIsCreating(false)
      setResultOpen(true)
    }
  }

  return (
    <div style={{ display: 'flex', height: '100%' }}>
      <ResizableSidebar>
        <Stack gap="xs">
          {/* Sub-tab switcher */}
          <div style={{
            display: 'flex',
            borderBottom: '1px solid var(--mantine-color-gray-3)',
            marginBottom: 4,
          }}>
            {(['select', 'new'] as MilestoneMode[]).map(m => (
              <button
                key={m}
                onClick={() => setMode(m)}
                style={{
                  padding: '4px 10px',
                  background: 'none',
                  border: 'none',
                  cursor: 'pointer',
                  fontSize: 13,
                  fontWeight: mode === m ? 600 : 400,
                  color: mode === m ? '#2f7a3b' : '#555',
                  borderBottom: mode === m ? '2px solid #2f7a3b' : '2px solid transparent',
                }}
              >
                {m === 'select' ? 'Select' : 'New'}
              </button>
            ))}
          </div>

          {/* Select sub-tab */}
          {mode === 'select' && (
            <Select
              label="Milestone"
              placeholder={isLoading ? 'Loading…' : 'Select a milestone'}
              size="xs"
              disabled={isLoading}
              data={[...openMilestones].reverse().map(ms => ({
                value: String(ms.number),
                label: ms.title,
                openIssues: ms.open_issues,
                closedIssues: ms.closed_issues,
              }))}
              value={selectedMilestone !== null ? String(selectedMilestone) : null}
              onChange={v => setSelectedMilestone(v !== null ? Number(v) : null)}
              nothingFoundMessage="No open milestones"
              searchable
              clearable
              renderOption={({ option }) => {
                const item = option as unknown as { label: string; openIssues: number; closedIssues: number }
                return (
                  <div>
                    <Text size="sm">{item.label}</Text>
                    <Text size="xs" c="dimmed">
                      {item.openIssues} open · {item.closedIssues} closed
                    </Text>
                  </div>
                )
              }}
            />
          )}

          {/* New sub-tab */}
          {mode === 'new' && (
            <>
              <TextInput
                label="Name"
                withAsterisk
                size="xs"
                value={newName}
                onChange={e => setNewName(e.currentTarget.value)}
                error={nameConflict ? 'Name already exists' : undefined}
                placeholder="e.g. Sprint 4"
              />
              <Textarea
                label="Description"
                size="xs"
                value={newDesc}
                onChange={e => setNewDesc(e.currentTarget.value)}
                placeholder="Optional"
                rows={3}
              />
              <Text size="xs" c="dimmed" mt={4}>
                The milestone will be created when you submit issues.
              </Text>
            </>
          )}
          <div style={{ marginTop: 8, display: 'flex', flexDirection: 'column', gap: 6 }}>
            <Tooltip label={queuedItems.length === 0 ? 'Queue at least one issue' : ''} withArrow disabled={queuedItems.length > 0} multiline maw={220}>
              <Button
                fullWidth
                size="sm"
                variant="default"
                disabled={queuedItems.length === 0}
                onClick={() => setBatchOpen(true)}
              >
                Batch Relevant Files
              </Button>
            </Tooltip>
            <Tooltip
              label={blockReason ?? createWarning?.tooltip ?? ''}
              withArrow
              color={!blockReason && createWarning ? createWarning.color : undefined}
              disabled={!blockReason && !createWarning}
              multiline
              maw={220}
            >
              <Button
                fullWidth
                size="sm"
                color={canCreate ? (createWarning?.color ?? '#2f9e44') : undefined}
                disabled={!canCreate || isCreating}
                loading={isCreating}
                onClick={handleCreate}
              >
                {`Create ${queuedItems.length} QC Issue${queuedItems.length !== 1 ? 's' : ''}`}
              </Button>
            </Tooltip>
          </div>
        </Stack>
      </ResizableSidebar>

      <BatchRelevantFilesModal
        opened={batchOpen}
        onClose={() => setBatchOpen(false)}
        queuedItems={queuedItems}
        onApply={handleBatchApply}
      />

      <CreateResultModal
        opened={resultOpen}
        outcome={createOutcome}
        onClose={() => setResultOpen(false)}
        onDone={(milestoneNumber) => {
          setResultOpen(false)
          setQueuedItems([])
          setMode('select')
          setSelectedMilestone(milestoneNumber)
          setNewName('')
          setNewDesc('')
        }}
      />

      <CreateIssueModal
        opened={modalOpen}
        onClose={() => { setModalOpen(false); setEditingIndex(null) }}
        milestoneNumber={mode === 'select' ? selectedMilestone : null}
        milestoneTitle={milestoneTitle}
        onQueue={(item) => setQueuedItems((prev) => [...prev, item])}
        onUpdate={(index, item) => setQueuedItems((prev) => prev.map((q, i) => i === index ? item : q))}
        queuedItems={queuedItems}
        editItem={editingIndex !== null ? (queuedItems[editingIndex] ?? null) : null}
        editIndex={editingIndex}
      />

      {/* Right main panel */}
      <div style={{ flex: 1, overflowY: 'auto', padding: 16 }}>
        {(mode === 'new' || mode === 'select') && (
          <>
            {mode === 'select' && issuesLoading && <Loader size="sm" />}
            <div style={{
              display: 'grid',
              gridTemplateColumns: 'repeat(auto-fill, minmax(240px, 1fr))',
              gridAutoRows: '180px',
              gap: 12,
            }}>
              <AddFileCard
                onClick={() => setModalOpen(true)}
                disabled={mode === 'select' && selectedMilestone === null}
              />
              {queuedItems.map((item, i) => (
                <QueuedIssueCard
                  key={i}
                  item={item}
                  onEdit={() => { setEditingIndex(i); setModalOpen(true) }}
                  onRemove={() => setQueuedItems((prev) => prev.filter((_, idx) => idx !== i))}
                  conflictReason={getConflictReason(item)}
                  dirty={dirtyFiles.has(item.file)}
                />
              ))}
              {mode === 'select' && milestoneIssues.map(issue => (
                <ExistingIssueCard key={issue.number} issue={issue} />
              ))}
            </div>
          </>
        )}
      </div>
    </div>
  )
}
