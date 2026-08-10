import { useState } from 'react'
import { ActionIcon, Collapse, Divider, Text, TextInput, Button, Textarea, Tooltip } from '@mantine/core'
import { IconChevronRight, IconRefresh } from '@tabler/icons-react'
import { useMutation, useQueryClient } from '@tanstack/react-query'
import {
  useConfigurationStatus,
  useChecklistDisplayName,
  setupConfiguration,
  updateConfiguration,
} from '~/api/configuration'
import type { ConfigGitRepository, ConfigurationStatus } from '~/api/configuration'
import { STATUS_COLOR } from './RepoStatus'
import { capitalize } from '~/utils/displayName'
import type { Checklist } from '~/api/checklists'
import { Splitter, useResizableWidth } from './ResizableSplitter'

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
  const localDirectoryOnly = configStatus.exists && git === null

  if (localDirectoryOnly) {
    return (
      <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
        <Text fw={700} size="sm">
          Not a git repository
        </Text>
        <Text size="xs" c="dimmed" style={{ fontFamily: 'monospace' }}>
          {configStatus.directory}
        </Text>
      </div>
    )
  }

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

  // A configured git repository is rendered by ConfigRepoStrip, which lives
  // outside the collapsible section so that it is always visible.
  return null
}

function StatusPill({ git }: { git: ConfigGitRepository }) {
  const color = STATUS_COLOR[git.status]
  return (
    <span
      style={{
        display: 'inline-flex',
        alignItems: 'center',
        gap: 4,
        backgroundColor: color + '22',
        border: `1px solid ${color}`,
        borderRadius: 10,
        padding: '1px 8px',
        fontSize: 12,
        color,
        fontWeight: 600,
      }}
    >
      <span
        style={{
          width: 7,
          height: 7,
          borderRadius: '50%',
          backgroundColor: color,
          display: 'inline-block',
        }}
      />
      {git.status}
    </span>
  )
}

/** Reason the Update button is unavailable, or null when an update can be run. */
function updateBlockedReason(git: ConfigGitRepository): string | null {
  if (git.dirty_files.length > 0) {
    return 'The configuration repository has uncommitted changes. Commit or discard them before updating.'
  }
  switch (git.status) {
    case 'clean':
      return 'The configuration repository is already up to date.'
    case 'ahead':
      return 'The configuration repository has local commits that are not on the remote. Push them before updating.'
    case 'diverged':
      return 'The configuration repository has diverged from its remote. Resolve it manually with git.'
    case 'behind':
      return null
  }
}

/**
 * Always-visible summary of the configuration repository's git state, with a
 * one-click update. Sits above the collapsible sections so a stale
 * configuration repository cannot be missed.
 */
function ConfigRepoStrip({
  configStatus,
  git,
}: {
  configStatus: ConfigurationStatus
  git: ConfigGitRepository
}) {
  const queryClient = useQueryClient()
  const { isFetching } = useConfigurationStatus()
  const mutation = useMutation({
    mutationFn: updateConfiguration,
    onSuccess: (data) => {
      queryClient.setQueryData(['configuration', 'status'], data)
      // Re-read everything derived from the configuration repository.
      void queryClient.invalidateQueries({ queryKey: ['configuration'] })
    },
  })

  const color = STATUS_COLOR[git.status]
  const blockedReason = updateBlockedReason(git)

  const updateButton = (
    <Button
      size="xs"
      onClick={() => mutation.mutate()}
      loading={mutation.isPending}
      disabled={blockedReason !== null}
    >
      Update
    </Button>
  )

  return (
    <div
      style={{
        display: 'flex',
        flexDirection: 'column',
        gap: 6,
        marginBottom: 16,
        padding: '10px 14px',
        borderRadius: 6,
        border: '1px solid var(--mantine-color-gray-3)',
        borderLeft: `3px solid ${color}`,
        backgroundColor: color + '0f',
      }}
    >
      <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
        <Text fw={700} size="sm">
          {git.owner} / {git.repo}
        </Text>
        <StatusPill git={git} />
        <Tooltip label="Re-check the configuration repository">
          <ActionIcon
            variant="subtle"
            size="sm"
            aria-label="Re-check repository status"
            loading={isFetching}
            onClick={() => void queryClient.invalidateQueries({ queryKey: ['configuration'] })}
          >
            <IconRefresh size={14} />
          </ActionIcon>
        </Tooltip>
        <div style={{ flex: 1 }} />
        {blockedReason ? (
          // A disabled Mantine Button swallows pointer events, so the tooltip
          // needs a wrapper element to attach to.
          <Tooltip label={blockedReason} multiline w={280} withArrow>
            <span style={{ display: 'inline-flex' }}>{updateButton}</span>
          </Tooltip>
        ) : (
          updateButton
        )}
      </div>

      {git.status_detail && (
        <Text size="xs" c="dimmed">
          {git.status_detail}
        </Text>
      )}

      <Text size="xs" c="dimmed" style={{ fontFamily: 'monospace' }}>
        {configStatus.directory}
      </Text>

      {git.dirty_files.length > 0 && (
        <Text size="xs" c="yellow.7">
          Dirty: {git.dirty_files.join(', ')}
        </Text>
      )}

      {mutation.isSuccess && (
        <Text size="xs" c="green.7">
          Configuration repository updated
        </Text>
      )}
      {mutation.isError && (
        <Text size="xs" c="red">
          {(mutation.error as Error).message}
        </Text>
      )}
    </div>
  )
}

function ChecklistsSection({ checklists }: { checklists: Checklist[] }) {
  const visible = checklists.filter((c) => c.name !== 'Custom')
  const [activeIndex, setActiveIndex] = useState(0)
  const active = visible[activeIndex]
  const { width: listWidth, onMouseDown: onSplitterDown, dragging } = useResizableWidth(140)

  if (visible.length === 0) return <Text size="sm" c="dimmed">No checklists found</Text>

  return (
    <div style={{ display: 'flex', gap: 8 }}>
      {/* Left: list */}
      <div style={{ display: 'flex', flexDirection: 'column', gap: 4, width: listWidth, flexShrink: 0 }}>
        {visible.map((c, i) => {
          const isActive = i === activeIndex
          return (
            <Tooltip key={c.name} label={c.name} openDelay={300} withArrow>
              <button
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
            </Tooltip>
          )
        })}
      </div>

      <Splitter onMouseDown={onSplitterDown} dragging={dragging} />

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
  const { singular } = useChecklistDisplayName()
  const singularCap = capitalize(singular)

  const rows: { label: string; value: React.ReactNode }[] = [
    { label: 'Display name', value: <Text size="sm">{opts.checklist_display_name}</Text> },
    {
      label: 'Include collaborators',
      value: <Text size="sm">{opts.include_collaborators ? 'Yes' : 'No'}</Text>,
    },
    {
      label: `${singularCap} directory`,
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
    {
      label: 'UI repo refresh rate',
      value: <Text size="sm">{opts.ui_repo_refresh_rate_seconds}s</Text>,
    },
    ...(opts.prepended_checklist_note !== null
      ? [{ label: `${singularCap} note`, value: <Text size="sm">{opts.prepended_checklist_note}</Text> }]
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
  const { plural } = useChecklistDisplayName()
  const pluralCap = capitalize(plural)

  if (isLoading || !configStatus) {
    return (
      <div style={{ maxWidth: 720, margin: 'auto', padding: 24 }}>
        <Text c="dimmed" size="sm">Loading configuration…</Text>
      </div>
    )
  }

  const git = configStatus.git_repository

  return (
    <div style={{ maxWidth: 720, margin: 'auto', padding: 24 }}>
      {git ? (
        <ConfigRepoStrip configStatus={configStatus} git={git} />
      ) : (
        <Section title="Git Repository">
          <GitRepoSection configStatus={configStatus} />
        </Section>
      )}

      <Section title={pluralCap}>
        <ChecklistsSection checklists={configStatus.checklists} />
      </Section>

      <Section title="Options">
        <OptionsSection configStatus={configStatus} />
      </Section>
    </div>
  )
}
