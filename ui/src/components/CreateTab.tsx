import { useState } from 'react'
import { Button, Select, Text, TextInput, Textarea, Stack, Loader, Tooltip } from '@mantine/core'
import { useMilestones } from '~/api/milestones'
import { useIssuesForMilestone } from '~/api/issues'
import { ResizableSidebar } from './ResizableSidebar'
import { ExistingIssueCard } from './ExistingIssueCard'
import { AddFileCard } from './AddFileCard'
import { CreateIssueModal } from './CreateIssueModal'
import type { QueuedItem } from './CreateIssueModal'
import { QueuedIssueCard } from './QueuedIssueCard'

type MilestoneMode = 'select' | 'new'

export function CreateTab() {
  const [mode, setMode] = useState<MilestoneMode>('select')
  const [selectedMilestone, setSelectedMilestone] = useState<number | null>(null)
  const [newName, setNewName] = useState('')
  const [newDesc, setNewDesc] = useState('')
  const [modalOpen, setModalOpen] = useState(false)
  const [editingIndex, setEditingIndex] = useState<number | null>(null)
  const [queuedItems, setQueuedItems] = useState<QueuedItem[]>([])
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
              data={openMilestones.map(ms => ({
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
          <div style={{ marginTop: 8 }}>
            <Tooltip label={blockReason ?? ''} withArrow disabled={!blockReason} multiline maw={220}>
              <Button
                fullWidth
                size="sm"
                disabled={!canCreate}
              >
                Create QC Issues
              </Button>
            </Tooltip>
          </div>
        </Stack>
      </ResizableSidebar>

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
