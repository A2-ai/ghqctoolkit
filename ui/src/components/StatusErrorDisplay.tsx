import { useState } from 'react'
import {
  ActionIcon,
  Anchor,
  Code,
  CopyButton,
  Divider,
  Popover,
  Stack,
  Text,
  Tooltip,
} from '@mantine/core'
import {
  IconAlertCircle,
  IconAlertTriangle,
  IconCheck,
  IconChevronDown,
  IconChevronUp,
  IconCopy,
} from '@tabler/icons-react'
import type { IssueStatusError, BlockingQCError } from '~/api/issues'
import { useRepoInfo } from '~/api/repo'

// Both error shapes share the fields we care about. Accept either.
type AnyStatusError = Pick<IssueStatusError | BlockingQCError, 'issue_number' | 'error'> & {
  kind?: IssueStatusError['kind']
  branch?: string
  file_name?: string
}

type Variant = 'icon-red' | 'icon-yellow' | 'inline-list'

interface Props {
  errors: AnyStatusError[]
  variant: Variant
}

const FALLBACK_REMOTE = 'origin'

interface BranchGroup {
  branch: string
  issues: number[]
}

function shellQuote(s: string): string {
  // POSIX-safe single-quote wrap. Embedded single quotes become '\''.
  return `'${s.replace(/'/g, `'\\''`)}'`
}

// Build a copy-pasteable command that creates the missing local branch refs
// without changing the working tree:
//   git fetch <remote>
//     && git branch --track 'foo' <remote>/foo
//     && git branch --track 'bar' <remote>/bar
//
// `--track` only sets up the local ref + upstream tracking — it does not
// switch the checkout — so we don't need to checkout back afterwards. The
// remote name comes from /api/repo (gix's find_default_remote).
function buildCheckoutCommand(branches: string[], remote: string): string | null {
  if (branches.length === 0) return null
  const parts = [`git fetch ${remote}`]
  for (const b of branches) {
    // We single-quote the local ref to handle spaces / shell-specials.
    // The `<remote>/<branch>` ref name uses the remote as parsed by gix —
    // it should be a plain identifier, but quote defensively.
    parts.push(`git branch --track ${shellQuote(b)} ${remote}/${b}`)
  }
  return parts.join(' && ')
}

// Extract the missing-branch name from an error. Falls back to a regex match
// on the legacy `"Branch not found: <name>"` string when `kind`/`branch` aren't
// populated (rolling-deploy of an older backend).
function extractMissingBranch(e: AnyStatusError): string | null {
  if (e.kind === 'branch_not_local') return e.branch ?? null
  const m = /^Branch not found: (.+)$/.exec(e.error)
  return m ? m[1] : null
}

function groupBranchErrors(errors: AnyStatusError[]): BranchGroup[] {
  const map = new Map<string, number[]>()
  for (const e of errors) {
    const branch = extractMissingBranch(e)
    if (branch === null) continue
    const list = map.get(branch) ?? []
    list.push(e.issue_number)
    map.set(branch, list)
  }
  return [...map.entries()]
    .map(([branch, issues]) => ({ branch, issues: [...new Set(issues)].sort((a, b) => a - b) }))
    .sort((a, b) => a.branch.localeCompare(b.branch))
}

function partition(errors: AnyStatusError[]): { branchErrors: AnyStatusError[]; otherErrors: AnyStatusError[] } {
  const branchErrors: AnyStatusError[] = []
  const otherErrors: AnyStatusError[] = []
  for (const e of errors) {
    if (extractMissingBranch(e) !== null) branchErrors.push(e)
    else otherErrors.push(e)
  }
  return { branchErrors, otherErrors }
}

// ---------------------------------------------------------------------------
// Tooltip-style summary list — always visible in the dropdown.
// ---------------------------------------------------------------------------
function ErrorSummaryList({ errors, dimmed }: { errors: AnyStatusError[]; dimmed?: boolean }) {
  return (
    <Stack gap={2}>
      {errors.map((e) => (
        <Text key={e.issue_number} size="xs" c={dimmed ? 'dimmed' : undefined}>
          #{e.issue_number}: {e.error}
        </Text>
      ))}
    </Stack>
  )
}

// `! <file_name> (#<n>) — <error>` — matches the approved/not_approved
// blocking-QC line format. Falls back to `#<n>: <error>` when there's no
// file_name (e.g. when the issue body itself was never fetched).
function InlineErrorLine({ error }: { error: AnyStatusError }) {
  if (error.file_name) {
    return (
      <Text size="sm" c="red">
        ! {error.file_name} (#{error.issue_number}) — {error.error}
      </Text>
    )
  }
  return (
    <Text size="xs" c="red">
      #{error.issue_number}: {error.error}
    </Text>
  )
}

// Inline label for the single-branch case, mirroring the approved/not_approved
// format when we know the file: `<file_name> (#<n>) — branch isn't checked out`.
function formatBranchInlineLabel(error: AnyStatusError): string {
  const suffix = "branch isn't checked out locally"
  if (error.file_name) {
    return `${error.file_name} (#${error.issue_number}) — ${suffix}`
  }
  return `Branch isn't checked out locally (#${error.issue_number})`
}

// ---------------------------------------------------------------------------
// Rich expanded section — branch list + copy-pasteable command.
// Renders `null` when there are no branch_not_local errors.
// ---------------------------------------------------------------------------
function BranchFixSection({ branchErrors }: { branchErrors: AnyStatusError[] }) {
  const { data: repoData } = useRepoInfo()
  const remote = repoData?.remote ?? FALLBACK_REMOTE
  const groups = groupBranchErrors(branchErrors)
  if (groups.length === 0) return null
  const branches = groups.map((g) => g.branch)
  const command = buildCheckoutCommand(branches, remote)

  return (
    <Stack gap="xs">
      <Text size="sm" fw={600}>These branches aren't checked out locally:</Text>
      <Stack gap={2}>
        {groups.map((g) => (
          <Text key={g.branch} size="sm">
            <Code>{g.branch}</Code>
            {' — '}
            {g.issues.map((n) => `#${n}`).join(', ')}
          </Text>
        ))}
      </Stack>

      {command && (
        <>
          <Text size="xs" c="dimmed">
            Run this to create the missing local refs:
          </Text>
          <div style={{ display: 'flex', alignItems: 'flex-start', gap: 6 }}>
            <Code block style={{ flex: 1, whiteSpace: 'pre-wrap', wordBreak: 'break-all' }}>
              {command}
            </Code>
            {/* Stop click bubbling so copying doesn't toggle the expanded state. */}
            <span onClick={(e) => e.stopPropagation()}>
              <CopyButton value={command} timeout={1500}>
                {({ copied, copy }) => (
                  <Tooltip label={copied ? 'Copied' : 'Copy command'} withArrow>
                    <ActionIcon
                      variant="subtle"
                      color={copied ? 'teal' : 'gray'}
                      onClick={copy}
                      aria-label="Copy command"
                    >
                      {copied ? <IconCheck size={16} /> : <IconCopy size={16} />}
                    </ActionIcon>
                  </Tooltip>
                )}
              </CopyButton>
            </span>
          </div>
        </>
      )}
    </Stack>
  )
}

// ---------------------------------------------------------------------------
// Public component
// ---------------------------------------------------------------------------
export function StatusErrorDisplay({ errors, variant }: Props) {
  // `expanded` controls whether the rich fix section is shown beneath the
  // basic per-issue summary. It's also used to keep the icon-variant dropdown
  // pinned open even when the user mouses out — otherwise clicking inside
  // the dropdown to expand would cause a flicker as the hover popover closed.
  const [expanded, setExpanded] = useState(false)
  const [hoverOpen, setHoverOpen] = useState(false)
  const [inlineOpen, setInlineOpen] = useState(false)

  if (errors.length === 0) return null

  const { branchErrors, otherErrors } = partition(errors)
  const hasBranchErrors = branchErrors.length > 0

  const toggleExpanded = (ev: React.MouseEvent) => {
    if (!hasBranchErrors) return
    ev.stopPropagation()
    setExpanded((e) => !e)
  }

  // -------- inline-list variant (used inside IssueDetailModal etc.) --------
  // Renders each error in the same shape as approved/not_approved blocking-QC
  // entries: `! <file_name> (#<n>) — <error/label>`. branch_not_local errors
  // get a clickable label that opens the fix popover.
  if (variant === 'inline-list') {
    return (
      <Stack gap={2}>
        {otherErrors.map((e) => (
          <InlineErrorLine key={e.issue_number} error={e} />
        ))}
        {hasBranchErrors && (
          <Popover
            opened={inlineOpen}
            onChange={setInlineOpen}
            position="bottom-start"
            shadow="md"
            withArrow
            withinPortal
          >
            <Popover.Target>
              <Tooltip label="Click to show fix" withArrow position="top-start">
                <Anchor
                  component="button"
                  type="button"
                  size="sm"
                  c="orange.7"
                  fw={500}
                  onClick={() => setInlineOpen((o) => !o)}
                  style={{
                    alignSelf: 'flex-start',
                    display: 'flex',
                    alignItems: 'flex-start',
                    gap: 6,
                    textAlign: 'left',
                  }}
                >
                  {/* Pin icon to first-line baseline; wrapped text aligns to
                      the text column, not under the icon. */}
                  <IconAlertTriangle
                    size={14}
                    style={{ flexShrink: 0, marginTop: 3 }}
                  />
                  <span>
                    {branchErrors.length === 1
                      ? formatBranchInlineLabel(branchErrors[0])
                      : `${branchErrors.length} blocking QCs aren't checked out locally`}
                  </span>
                </Anchor>
              </Tooltip>
            </Popover.Target>
            <Popover.Dropdown>
              <BranchFixSection branchErrors={branchErrors} />
            </Popover.Dropdown>
          </Popover>
        )}
      </Stack>
    )
  }

  // -------- icon variants — milestone-level surfaces ----------------------
  const isRed = variant === 'icon-red'
  const Icon = isRed ? IconAlertCircle : IconAlertTriangle
  const color = isRed ? '#c92a2a' : '#e67700'
  const ChevronIcon = expanded ? IconChevronUp : IconChevronDown

  // The dropdown body. Always shows the basic per-issue list (the original
  // tooltip content); when `expanded`, the rich fix section appears beneath.
  // Clicking anywhere on the body toggles `expanded` (when there's a fix to
  // show); the copy button stops propagation so it doesn't trigger toggle.
  const dropdownContent = (
    <div
      onClick={toggleExpanded}
      style={{
        cursor: hasBranchErrors ? 'pointer' : 'default',
        maxWidth: 520,
      }}
    >
      <Stack gap="xs">
        <ErrorSummaryList errors={errors} />
        {hasBranchErrors && (
          <>
            <Divider />
            {expanded ? (
              <BranchFixSection branchErrors={branchErrors} />
            ) : (
              <Text
                size="xs"
                c="dimmed"
                style={{ display: 'flex', alignItems: 'center', gap: 4 }}
              >
                <ChevronIcon size={12} />
                Click to show fix
              </Text>
            )}
          </>
        )}
      </Stack>
    </div>
  )

  // `opened` is the OR of "user is hovering the trigger or dropdown" and
  // "user clicked to expand". Hover is the cheap preview; clicking pins the
  // popover so interacting with its contents (selecting text, hitting copy)
  // doesn't dismiss it. When the user collapses again, hover regains control.
  const opened = hoverOpen || expanded

  const trigger = (
    <span
      data-testid={isRed ? 'status-error-count' : 'partial-warning'}
      onClick={(ev) => {
        if (!hasBranchErrors) return
        ev.stopPropagation()
        setExpanded((e) => !e)
      }}
      onMouseEnter={() => setHoverOpen(true)}
      onMouseLeave={() => setHoverOpen(false)}
      style={{
        color,
        display: 'flex',
        alignItems: 'center',
        gap: 2,
        flexShrink: 0,
        cursor: hasBranchErrors ? 'pointer' : 'default',
      }}
    >
      <Icon size={14} />
      {errors.length}
    </span>
  )

  return (
    <Popover
      opened={opened}
      onChange={(o) => {
        if (!o) {
          setHoverOpen(false)
          setExpanded(false)
        }
      }}
      position="bottom"
      shadow="md"
      withArrow
      withinPortal
      closeOnClickOutside
    >
      <Popover.Target>{trigger}</Popover.Target>
      <Popover.Dropdown
        onMouseEnter={() => setHoverOpen(true)}
        onMouseLeave={() => setHoverOpen(false)}
      >
        {dropdownContent}
      </Popover.Dropdown>
    </Popover>
  )
}
