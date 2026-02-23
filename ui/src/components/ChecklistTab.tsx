import { useEffect, useRef, useState } from 'react'
import { Alert, Button, Group, Loader, Stack, Text, TextInput, Textarea } from '@mantine/core'
import { fetchChecklists } from '~/api/checklists'

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
}

export function ChecklistTab({ onChange }: Props) {
  const counter = useRef(0)

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

        // Activate the first visible tab
        const first = entries[0]
        if (first) {
          setActiveKey(first.key)
          setEditorName(first.savedName)
          setEditorContent(first.savedContent)
          onChange({ name: first.savedName, content: first.savedContent })
        }
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
    setEditorName(tab.originalName)
    setEditorContent(tab.originalContent)
    onChange({ name: tab.originalName, content: tab.originalContent })
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
              onClick={() => loadTab(tab.key)}
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
        <TextInput
          label="Name"
          value={editorName}
          onChange={(e) => handleNameChange(e.currentTarget.value)}
        />
        <div>
          <Text size="sm" fw={500} mb={4}>
            Checklist
          </Text>
          <Textarea
            value={editorContent}
            onChange={(e) => handleContentChange(e.currentTarget.value)}
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
      </Stack>
    </div>
  )
}
