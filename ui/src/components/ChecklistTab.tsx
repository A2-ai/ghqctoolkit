import { useEffect, useRef, useState } from 'react'
import { Alert, Badge, Button, Group, Loader, Stack, Text, TextInput, Tooltip } from '@mantine/core'
import { CommentEditor } from './CommentEditor'
import { fetchChecklists } from '~/api/checklists'
import { useChecklistDisplayName } from '~/api/configuration'
import { capitalize } from '~/utils/displayName'
import { Splitter, useResizableWidth } from './ResizableSplitter'

export interface ChecklistDraft {
  name: string
  content: string
}

interface TabEntry {
  key: string
  isQueuedSnapshot?: boolean
  // Editor-bound, modal-session-only. Mirrors keystrokes; survives tab switches.
  draftName: string
  draftContent: string
  // Last persisted across modal opens (via onSaveCustom).
  savedName: string
  savedContent: string
  // Immutable API default; used by Reset for API-origin tabs.
  originalName: string
  originalContent: string
}

interface Props {
  onChange: (draft: ChecklistDraft) => void
  onSelect?: () => void
  initialDraft?: ChecklistDraft | null
  /** Custom tabs saved in previous modal opens; restored on remount */
  persistedCustomTabs?: ChecklistDraft[]
  /** Called when user saves a custom (non-API) tab; parent stores it across opens */
  onSaveCustom?: (tab: ChecklistDraft) => void
}

const QUEUED_SNAPSHOT_KEY = 'queued-snapshot'

export function ChecklistTab({ onChange, onSelect, initialDraft, persistedCustomTabs, onSaveCustom }: Props) {
  const counter = useRef(0)
  const { singular } = useChecklistDisplayName()
  const singularCap = capitalize(singular)

  const [tabs, setTabs] = useState<TabEntry[]>([])
  const [activeKey, setActiveKey] = useState<string | null>(null)
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)

  // Editor state — reflects what's currently in the fields (may be unsaved)
  const [editorName, setEditorName] = useState('')
  const [editorContent, setEditorContent] = useState('')

  const { width: listWidth, onMouseDown: onSplitterDown, dragging } = useResizableWidth(140)

  // Custom template stored for "+ New"
  const customRef = useRef<{ name: string; content: string } | null>(null)

  useEffect(() => {
    fetchChecklists()
      .then((data) => {
        const custom = data.find((t) => t.name === 'Custom')
        if (custom) customRef.current = { name: custom.name, content: custom.content }

        // Exclude "Custom" from the visible tab list — it's only used as the "+ New" seed
        const visible = data.filter((t) => t.name !== 'Custom')
        const apiNames = new Set(visible.map((t) => t.name))
        const persistedMap = new Map((persistedCustomTabs ?? []).map((t) => [t.name, t]))

        // API tabs, possibly with persisted-save overrides on saved fields.
        const apiEntries: TabEntry[] = visible.map((t, i) => {
          const persisted = persistedMap.get(t.name)
          const savedName = persisted ? persisted.name : t.name
          const savedContent = persisted ? persisted.content : t.content
          return {
            key: `api-${i}`,
            draftName: savedName,
            draftContent: savedContent,
            savedName,
            savedContent,
            originalName: t.name,
            originalContent: t.content,
          }
        })

        // Persisted custom (non-API) tabs.
        const extraEntries: TabEntry[] = (persistedCustomTabs ?? [])
          .filter((t) => !apiNames.has(t.name))
          .map((t, i) => ({
            key: `persisted-${i}`,
            draftName: t.name,
            draftContent: t.content,
            savedName: t.name,
            savedContent: t.content,
            originalName: t.name,
            originalContent: t.content,
          }))

        let allEntries: TabEntry[] = [...apiEntries, ...extraEntries]

        // Edit mode: prepend a queued-snapshot tab and activate it.
        if (initialDraft) {
          const snapshotTab: TabEntry = {
            key: QUEUED_SNAPSHOT_KEY,
            isQueuedSnapshot: true,
            draftName: initialDraft.name,
            draftContent: initialDraft.content,
            savedName: initialDraft.name,
            savedContent: initialDraft.content,
            originalName: initialDraft.name,
            originalContent: initialDraft.content,
          }
          allEntries = [snapshotTab, ...allEntries]
          setTabs(allEntries)
          setLoading(false)
          setActiveKey(QUEUED_SNAPSHOT_KEY)
          setEditorName(initialDraft.name)
          setEditorContent(initialDraft.content)
          onChange({ name: initialDraft.name, content: initialDraft.content })
          return
        }

        setTabs(allEntries)
        setLoading(false)
        // Fresh create: no tab selected, no editor rendered
        setActiveKey(null)
      })
      .catch((err: Error) => {
        setError(err.message)
        setLoading(false)
      })
  }, [])

  function loadTab(key: string, entries = tabs) {
    const tab = entries.find((t) => t.key === key)
    if (!tab) return
    setActiveKey(key)
    setEditorName(tab.draftName)
    setEditorContent(tab.draftContent)
    onChange({ name: tab.draftName, content: tab.draftContent })
  }

  function updateActiveDraft(name: string, content: string) {
    if (!activeKey) return
    setTabs((prev) =>
      prev.map((t) => (t.key === activeKey ? { ...t, draftName: name, draftContent: content } : t)),
    )
  }

  function handleNameChange(val: string) {
    setEditorName(val)
    updateActiveDraft(val, editorContent)
    onChange({ name: val, content: editorContent })
  }

  function handleContentChange(val: string) {
    setEditorContent(val)
    updateActiveDraft(editorName, val)
    onChange({ name: editorName, content: val })
  }

  function handleSave() {
    if (!activeKey) return
    setTabs((prev) =>
      prev.map((t) =>
        t.key === activeKey
          ? { ...t, draftName: editorName, draftContent: editorContent, savedName: editorName, savedContent: editorContent }
          : t,
      ),
    )
    onSaveCustom?.({ name: editorName, content: editorContent })
  }

  function handleReset() {
    const tab = tabs.find((t) => t.key === activeKey)
    if (!tab) return
    setEditorName(tab.savedName)
    setEditorContent(tab.savedContent)
    updateActiveDraft(tab.savedName, tab.savedContent)
    onChange({ name: tab.savedName, content: tab.savedContent })
  }

  function handleNew() {
    const key = `new-${++counter.current}`
    const src = customRef.current ?? { name: 'Custom', content: '' }
    const newTab: TabEntry = {
      key,
      draftName: src.name,
      draftContent: src.content,
      savedName: src.name,
      savedContent: src.content,
      originalName: src.name,
      originalContent: src.content,
    }
    setTabs((prev) => {
      const next = [...prev, newTab]
      loadTab(key, next)
      return next
    })
    onSelect?.()
  }

  if (loading) return <Loader size="sm" />
  if (error) return <Alert color="red">{error}</Alert>

  return (
    <div style={{ display: 'flex', gap: 8 }}>
      {/* Left: vertical tab list */}
      <div style={{ display: 'flex', flexDirection: 'column', gap: 4, width: listWidth, flexShrink: 0 }}>
        {tabs.map((tab) => {
          const active = tab.key === activeKey
          return (
            <Tooltip
              key={tab.key}
              label={tab.isQueuedSnapshot ? `${tab.draftName} (queued snapshot)` : tab.draftName}
              openDelay={300}
              withArrow
            >
              <button
                onClick={() => { loadTab(tab.key); onSelect?.() }}
                style={{
                  textAlign: 'left',
                  padding: '6px 10px',
                  borderRadius: 4,
                  border: `1px solid ${active ? '#2f9e44' : 'var(--mantine-color-gray-3)'}`,
                  background: active ? '#ebfbee' : 'white',
                  cursor: 'pointer',
                  fontWeight: active ? 600 : 400,
                  color: active ? '#2b8a3e' : 'inherit',
                  overflow: 'hidden',
                  textOverflow: 'ellipsis',
                  whiteSpace: 'nowrap',
                  fontSize: 13,
                  display: 'flex',
                  alignItems: 'center',
                  gap: 6,
                }}
              >
                <span style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', flex: 1 }}>
                  {tab.draftName}
                </span>
                {tab.isQueuedSnapshot && (
                  <Badge size="xs" variant="light" color="blue" style={{ flexShrink: 0 }}>
                    queued
                  </Badge>
                )}
              </button>
            </Tooltip>
          )
        })}
        <button
          onClick={handleNew}
          style={{
            textAlign: 'left',
            padding: '6px 10px',
            borderRadius: 4,
            border: '1px dashed var(--mantine-color-gray-4)',
            background: 'none',
            cursor: 'pointer',
            fontSize: 13,
            color: 'var(--mantine-color-gray-6)',
          }}
        >
          + New
        </button>
      </div>

      <Splitter onMouseDown={onSplitterDown} dragging={dragging} />

      {/* Right: editor */}
      <Stack gap="sm" style={{ flex: 1, minWidth: 0 }}>
        {activeKey === null && (
          <Text size="sm" c="dimmed" mt="xs">
            Select a {singular} from the list, or click + New to create one.
          </Text>
        )}
        {activeKey !== null && <TextInput
          label="Name"
          value={editorName}
          onChange={(e) => handleNameChange(e.currentTarget.value)}
        />}
        {activeKey !== null && (
          <>
            <div>
              <Text size="sm" fw={500} mb={4}>
                {singularCap}
              </Text>
              <CommentEditor
                value={editorContent}
                onChange={handleContentChange}
                monospace
                minHeight={200}
                onKeyDown={(e) => {
                  if (e.key === 'Tab') {
                    e.preventDefault()
                    const el = e.currentTarget
                    const start = el.selectionStart
                    const end = el.selectionEnd
                    const next = editorContent.slice(0, start) + '  ' + editorContent.slice(end)
                    handleContentChange(next)
                    requestAnimationFrame(() => {
                      el.selectionStart = el.selectionEnd = start + 2
                    })
                  }
                }}
              />
            </div>
            <Group justify="flex-end" gap="xs">
              <Button variant="default" size="xs" onClick={handleReset}>
                Reset
              </Button>
              <Button variant="default" size="xs" onClick={handleSave}>
                Save
              </Button>
            </Group>
          </>
        )}
      </Stack>
    </div>
  )
}
