import { useEffect, useRef, useState } from 'react'
import { Alert, Button, Group, Loader, Stack, Text, TextInput, Textarea } from '@mantine/core'
import { fetchChecklists } from '~/api/checklists'
import { useChecklistDisplayName } from '~/api/configuration'
import { capitalize } from '~/utils/displayName'

export interface ChecklistDraft {
  name: string
  content: string
}

interface TabEntry {
  key: string
  savedName: string
  savedContent: string
  originalName: string    // immutable API default — used by Reset
  originalContent: string
}

interface Props {
  onChange: (draft: ChecklistDraft) => void
  onSelect?: () => void
  initialDraft?: ChecklistDraft | null
}

export function ChecklistTab({ onChange, onSelect, initialDraft }: Props) {
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

  // Custom template stored for "+ New"
  const customRef = useRef<{ name: string; content: string } | null>(null)

  useEffect(() => {
    fetchChecklists()
      .then((data) => {
        const custom = data.find((t) => t.name === 'Custom')
        if (custom) customRef.current = { name: custom.name, content: custom.content }

        // Exclude "Custom" from the visible tab list — it's only used as the "+ New" seed
        const visible = data.filter((t) => t.name !== 'Custom')
        const entries: TabEntry[] = visible.map((t, i) => ({
          key: `api-${i}`,
          savedName: t.name,
          savedContent: t.content,
          originalName: t.name,
          originalContent: t.content,
        }))

        setTabs(entries)
        setLoading(false)

        // When editing, pre-select the matching tab (or create a custom one)
        if (initialDraft) {
          const match = entries.find((e) => e.savedName === initialDraft.name)
          if (match) {
            setActiveKey(match.key)
            setEditorName(match.savedName)
            setEditorContent(match.savedContent)
            onChange({ name: match.savedName, content: match.savedContent })
          } else {
            const customKey = 'edit-custom'
            const customTab: TabEntry = {
              key: customKey,
              savedName: initialDraft.name,
              savedContent: initialDraft.content,
              originalName: initialDraft.name,
              originalContent: initialDraft.content,
            }
            setTabs([...entries, customTab])
            setActiveKey(customKey)
            setEditorName(initialDraft.name)
            setEditorContent(initialDraft.content)
            onChange({ name: initialDraft.name, content: initialDraft.content })
          }
          return
        }

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
    setEditorName(tab.savedName)
    setEditorContent(tab.savedContent)
    onChange({ name: tab.savedName, content: tab.savedContent })
  }

  function handleNameChange(val: string) {
    setEditorName(val)
    onChange({ name: val, content: editorContent })
  }

  function handleContentChange(val: string) {
    setEditorContent(val)
    onChange({ name: editorName, content: val })
  }

  function handleSave() {
    if (!activeKey) return
    setTabs((prev) =>
      prev.map((t) =>
        t.key === activeKey ? { ...t, savedName: editorName, savedContent: editorContent } : t,
      ),
    )
  }

  function handleReset() {
    const tab = tabs.find((t) => t.key === activeKey)
    if (!tab) return
    setEditorName(tab.savedName)
    setEditorContent(tab.savedContent)
    onChange({ name: tab.savedName, content: tab.savedContent })
  }

  function handleNew() {
    const key = `new-${++counter.current}`
    const src = customRef.current ?? { name: 'Custom', content: '' }
    const newTab: TabEntry = {
      key,
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
    <div style={{ display: 'flex', gap: 16 }}>
      {/* Left: vertical tab list */}
      <div style={{ display: 'flex', flexDirection: 'column', gap: 4, width: 140, flexShrink: 0 }}>
        {tabs.map((tab) => {
          const active = tab.key === activeKey
          return (
            <button
              key={tab.key}
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
              }}
            >
              {tab.savedName}
            </button>
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
              <Textarea
                value={editorContent}
                onChange={(e) => handleContentChange(e.currentTarget.value)}
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
                autosize
                minRows={8}
                maxRows={14}
                styles={{ input: { fontFamily: 'monospace', fontSize: 12 } }}
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
