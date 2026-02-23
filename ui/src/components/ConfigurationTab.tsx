import { useState } from 'react'
import { Collapse, Divider, Text, TextInput, Button, Textarea } from '@mantine/core'
import { IconChevronRight } from '@tabler/icons-react'
import { useMutation, useQueryClient } from '@tanstack/react-query'
import { useConfigurationStatus, setupConfiguration } from '~/api/configuration'
import type { ConfigurationStatus } from '~/api/configuration'
import type { Checklist } from '~/api/checklists'

function Section({
  title,
  children,
  defaultOpen = true,
}: {
  title: string
  children: React.ReactNode
  defaultOpen?: boolean
}) {
  const [open, setOpen] = useState(defaultOpen)
  return (
    <div style={{ marginBottom: 16 }}>
      <button
        onClick={() => setOpen((o) => !o)}
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 6,
          width: '100%',
          background: 'none',
          border: 'none',
          cursor: 'pointer',
          padding: '8px 0',
          fontSize: 15,
          fontWeight: 600,
        }}
      >
        <IconChevronRight
          size={16}
          style={{ transform: open ? 'rotate(90deg)' : 'none', transition: 'transform 150ms', flexShrink: 0 }}
        />
        {title}
      </button>
      <Divider mb={open ? 12 : 0} />
      <Collapse in={open}>{children}</Collapse>
    </div>
  )
}

function GitRepoSection({ configStatus }: { configStatus: ConfigurationStatus }) {
  const queryClient = useQueryClient()
  const envUrl = configStatus.config_repo_env
  const [url, setUrl] = useState(envUrl ?? '')

  const mutation = useMutation({
    mutationFn: () => setupConfiguration(envUrl ?? url),
    onSuccess: (data) => {
      queryClient.setQueryData(['configuration', 'status'], data)
    },
  })

  const git = configStatus.git_repository

  if (!git) {
    return (
      <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
        <div style={{ display: 'flex', gap: 8, alignItems: 'flex-end' }}>
          <div style={{ flex: 1 }}>
            <TextInput
              placeholder="https://github.com/owner/config-repo"
              value={envUrl ?? url}
              onChange={envUrl ? undefined : (e) => setUrl(e.currentTarget.value)}
              label="Git URL"
              disabled={!!envUrl || mutation.isPending}
            />
            {envUrl && (
              <Text size="xs" c="dimmed" mt={4}>
                Set by GHQC_CONFIG_REPO
              </Text>
            )}
          </div>
          <Button
            onClick={() => mutation.mutate()}
            loading={mutation.isPending}
            disabled={!(envUrl ?? url).trim()}
            style={{ marginBottom: envUrl ? 22 : 0 }}
          >
            Set Up
          </Button>
        </div>
        {mutation.isError && (
          <Text c="red" size="sm">
            {(mutation.error as Error).message}
          </Text>
        )}
      </div>
    )
  }

  const statusColor =
    git.status === 'clean'
      ? '#2f9e44'
      : git.status === 'diverged'
        ? '#c92a2a'
        : '#e67700'

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
      <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
        <Text fw={700} size="sm">
          {git.owner} / {git.repo}
        </Text>
        <span
          style={{
            display: 'inline-flex',
            alignItems: 'center',
            gap: 4,
            backgroundColor: statusColor + '22',
            border: `1px solid ${statusColor}`,
            borderRadius: 10,
            padding: '1px 8px',
            fontSize: 12,
            color: statusColor,
            fontWeight: 600,
          }}
        >
          <span
            style={{
              width: 7,
              height: 7,
              borderRadius: '50%',
              backgroundColor: statusColor,
              display: 'inline-block',
            }}
          />
          {git.status}
        </span>
      </div>
      <Text size="xs" c="dimmed" style={{ fontFamily: 'monospace' }}>
        {configStatus.directory}
      </Text>
      {git.dirty_files.length > 0 && (
        <Text size="xs" c="yellow.7">
          Dirty: {git.dirty_files.join(', ')}
        </Text>
      )}
    </div>
  )
}

function ChecklistsSection({ checklists }: { checklists: Checklist[] }) {
  const visible = checklists.filter((c) => c.name !== 'Custom')
  const [activeIndex, setActiveIndex] = useState(0)
  const active = visible[activeIndex]

  if (visible.length === 0) return <Text size="sm" c="dimmed">No checklists found</Text>

  return (
    <div style={{ display: 'flex', gap: 16 }}>
      {/* Left: list */}
      <div style={{ display: 'flex', flexDirection: 'column', gap: 4, width: 140, flexShrink: 0 }}>
        {visible.map((c, i) => {
          const isActive = i === activeIndex
          return (
            <button
              key={c.name}
              onClick={() => setActiveIndex(i)}
              style={{
                textAlign: 'left',
                padding: '6px 10px',
                borderRadius: 4,
                border: `1px solid ${isActive ? '#2f9e44' : 'var(--mantine-color-gray-3)'}`,
                background: isActive ? '#ebfbee' : 'white',
                cursor: 'pointer',
                fontWeight: isActive ? 600 : 400,
                color: isActive ? '#2b8a3e' : 'inherit',
                overflow: 'hidden',
                textOverflow: 'ellipsis',
                whiteSpace: 'nowrap',
                fontSize: 13,
              }}
            >
              {c.name}
            </button>
          )
        })}
      </div>

      {/* Right: read-only content */}
      {active && (
        <div style={{ flex: 1, minWidth: 0 }}>
          <Text fw={600} mb={8} size="sm">
            {active.name}
          </Text>
          <Textarea
            value={active.content}
            readOnly
            styles={{ input: { fontFamily: 'monospace', fontSize: 12, height: 320, overflowY: 'auto', resize: 'vertical' } }}
          />
        </div>
      )}
    </div>
  )
}

function OptionsSection({ configStatus }: { configStatus: ConfigurationStatus }) {
  const opts = configStatus.options

  const rows: { label: string; value: React.ReactNode }[] = [
    { label: 'Display name', value: <Text size="sm">{opts.checklist_display_name}</Text> },
    {
      label: 'Checklist directory',
      value: (
        <Text size="sm" style={{ fontFamily: 'monospace' }}>
          {opts.checklist_directory}
        </Text>
      ),
    },
    {
      label: 'Logo path',
      value: (
        <span style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
          <Text size="sm" style={{ fontFamily: 'monospace' }}>
            {opts.logo_path}
          </Text>
          <Text size="sm" c={opts.logo_found ? 'green' : 'red'} fw={700}>
            {opts.logo_found ? '✓' : '✗'}
          </Text>
        </span>
      ),
    },
    {
      label: 'Record path',
      value: (
        <Text size="sm" style={{ fontFamily: 'monospace' }}>
          {opts.record_path}
        </Text>
      ),
    },
    ...(opts.prepended_checklist_note !== null
      ? [{ label: 'Checklist note', value: <Text size="sm">{opts.prepended_checklist_note}</Text> }]
      : []),
  ]

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
      {rows.map(({ label, value }) => (
        <div key={label} style={{ display: 'flex', alignItems: 'baseline', gap: 8 }}>
          <Text c="dimmed" size="sm" style={{ minWidth: 160, flexShrink: 0 }}>
            {label}
          </Text>
          {value}
        </div>
      ))}
    </div>
  )
}

export function ConfigurationTab() {
  const { data: configStatus, isLoading } = useConfigurationStatus()

  if (isLoading || !configStatus) {
    return (
      <div style={{ maxWidth: 720, margin: 'auto', padding: 24 }}>
        <Text c="dimmed" size="sm">Loading configuration…</Text>
      </div>
    )
  }

  return (
    <div style={{ maxWidth: 720, margin: 'auto', padding: 24 }}>
      <Section title="Git Repository">
        <GitRepoSection configStatus={configStatus} />
      </Section>

      <Section title="Checklists">
        <ChecklistsSection checklists={configStatus.checklists} />
      </Section>

      <Section title="Options">
        <OptionsSection configStatus={configStatus} />
      </Section>
    </div>
  )
}
