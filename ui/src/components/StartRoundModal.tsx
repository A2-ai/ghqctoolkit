import { useState } from 'react'
import {
  Alert,
  Anchor,
  Badge,
  Button,
  Group,
  Loader,
  Modal,
  SegmentedControl,
  Select,
  Stack,
  Tabs,
  Text,
  TextInput,
} from '@mantine/core'
import { IconAlertTriangle, IconInfoCircle } from '@tabler/icons-react'
import {
  isNothingToRepairError,
  isRoundStillOpenError,
  useRepairRound,
  useRoundSeed,
  useStartRound,
  type ImpactedIssues,
  type NotificationMode,
  type RepairRoundResponse,
  type RoundRepairStatus,
  type RoundSeedResponse,
  type StartRoundResponse,
  type StepOutcome,
} from '~/api/rounds'
import { useQuery } from '@tanstack/react-query'
import { commitDiffQueryKey, fetchCommitDiff } from '~/api/commits'
import { CommentEditor } from './CommentEditor'

export interface StartRoundModalProps {
  /** The issue to start a new QC round for; `null` keeps the modal closed. */
  issueNumber: number | null
  /** File name of the QC'd file, shown as context in the header. Optional. */
  issueTitle?: string
  /** The issue's GitHub URL, so the header line links to it. Optional. */
  issueUrl?: string
  /**
   * `round_repair` from the issue's status: which of the open round's follow-up
   * steps are incomplete. When it reports `needs_repair`, the modal offers a repair
   * instead of leaving the user with nothing to do but read `blocked_reason`.
   */
  repair?: RoundRepairStatus | null
  onClose: () => void
}

/**
 * Start-new-round modal (S4).
 *
 * Seeds itself from `GET /issues/{n}/rounds/seed`, which is also the source of
 * truth for whether a round *may* be started at all — when `can_start` is false
 * the action is not offered and the backend's `blocked_reason` is rendered
 * verbatim (a round already open, or an anchor that could not be resolved).
 */
export function StartRoundModal({ issueNumber, issueTitle, issueUrl, repair, onClose }: StartRoundModalProps) {
  return (
    <Modal
      opened={issueNumber !== null}
      onClose={onClose}
      title="Start New QC Round"
      size={720}
      centered
      styles={{ header: { paddingTop: 12, paddingBottom: 12 }, body: { paddingBottom: 20 } }}
    >
      {issueNumber !== null && (
        // Keyed so switching issues remounts with fresh form + mutation state.
        <StartRoundBody
          key={issueNumber}
          issueNumber={issueNumber}
          issueTitle={issueTitle}
          issueUrl={issueUrl}
          repair={repair}
          onClose={onClose}
        />
      )}
    </Modal>
  )
}

function StartRoundBody({
  issueNumber,
  issueTitle,
  issueUrl,
  repair,
  onClose,
}: {
  issueNumber: number
  issueTitle?: string
  issueUrl?: string
  repair?: RoundRepairStatus | null
  onClose: () => void
}) {
  const seedQuery = useRoundSeed(issueNumber)

  if (seedQuery.isPending) {
    return (
      <Group gap="xs" data-testid="round-seed-loading">
        <Loader size="sm" />
        <Text size="sm" c="dimmed">Loading round details…</Text>
      </Group>
    )
  }

  if (seedQuery.error) {
    return (
      <Stack gap="sm">
        <Alert color="red" title="Could not load round details" data-testid="round-seed-error">
          <Text size="sm">{seedQuery.error.message}</Text>
        </Alert>
        <Group justify="flex-end">
          <Button variant="default" onClick={onClose}>Close</Button>
        </Group>
      </Stack>
    )
  }

  return (
    <StartRoundForm
      issueNumber={issueNumber}
      issueTitle={issueTitle}
      issueUrl={issueUrl}
      seed={seedQuery.data}
      repair={repair}
      onClose={onClose}
    />
  )
}

/**
 * The form's three concerns, in the order the work happens: what changed since the
 * last approval, what to review it against, and who hears about it.
 */
type FormTab = 'changes' | 'checklist' | 'notification'

const NOTIFICATION_OPTIONS: { value: NotificationMode; label: string; description: string }[] = [
  {
    value: 'full',
    label: 'Full',
    description:
      'Posts a QC Notification comment with the inline diff of what changed since the previous approval.',
  },
  {
    value: 'metadata_only',
    label: 'Metadata only',
    description: 'Posts the QC Notification comment, but without the inline diff.',
  },
  {
    value: 'none',
    label: 'No notification',
    description: 'Opens the round silently — no notification comment is posted.',
  },
]

function StartRoundForm({
  issueNumber,
  issueTitle,
  issueUrl,
  seed,
  repair,
  onClose,
}: {
  issueNumber: number
  issueTitle?: string
  issueUrl?: string
  seed: RoundSeedResponse
  repair?: RoundRepairStatus | null
  onClose: () => void
}) {
  const [checklistContent, setChecklistContent] = useState(seed.checklist_content ?? '')
  const [checklistName, setChecklistName] = useState(seed.checklist_name ?? '')
  // Which round's checklist the editor was seeded from. Rounds diverge, so the
  // newest is only the default, not the only sensible base.
  const [sourceRound, setSourceRound] = useState<string | null>(
    seed.default_round === null ? null : String(seed.default_round),
  )
  const [note, setNote] = useState('')
  const [notification, setNotification] = useState<NotificationMode>('full')
  const [tab, setTab] = useState<FormTab>('checklist')
  const startRound = useStartRound(issueNumber)

  // A 201 is a success even when it reports failed steps — see StartRoundResultPanel.
  if (startRound.data) {
    return (
      <StartRoundResultPanel
        issueNumber={issueNumber}
        result={startRound.data}
        notificationMode={notification}
        onClose={onClose}
      />
    )
  }

  const noPriorChecklist = seed.checklist_content === null
  const canSubmit = seed.can_start && checklistContent.trim().length > 0
  const selectedOption = NOTIFICATION_OPTIONS.find((o) => o.value === notification)!
  // The expected 409 precondition: a round is already open, so nothing was posted.
  const roundAlreadyOpen: boolean = startRound.error !== null && isRoundStillOpenError(startRound.error)

  /**
   * Re-seed the editor from another round. Replacing the content outright is the
   * point of the control — the author asked to base this round on that one — so
   * edits made before switching are deliberately discarded.
   */
  function selectSourceRound(value: string | null) {
    setSourceRound(value)
    const option = seed.checklist_options.find((o) => String(o.round) === value)
    if (!option) return
    setChecklistContent(option.content)
    setChecklistName(option.checklist_name ?? '')
  }

  function handleSubmit() {
    startRound.mutate({
      checklist_content: checklistContent,
      checklist_name: checklistName.trim() === '' ? null : checklistName.trim(),
      note: note.trim() === '' ? null : note.trim(),
      notification,
    })
  }

  return (
    <Stack gap="md">
      <Stack gap={2}>
        <Text size="sm" fw={700} data-testid="next-round-name">{seed.next_round_name}</Text>
        {/*
          Linked so the issue is one click away — a reviewer opening a round often
          wants the thread it belongs to.
        */}
        {issueUrl ? (
          <Anchor
            href={issueUrl}
            target="_blank"
            rel="noreferrer"
            size="xs"
            data-testid="round-issue-link"
          >
            #{issueNumber} · {issueTitle ?? seed.file}
          </Anchor>
        ) : (
          <Text size="xs" c="dimmed">#{issueNumber} · {issueTitle ?? seed.file}</Text>
        )}
        <Text size="xs" c="dimmed" mt={2} data-testid="new-round-guidance">
          The previous round's approval stays valid — nothing downstream is affected.
        </Text>
      </Stack>

      {!seed.can_start && (
        <Alert
          color="orange"
          icon={<IconAlertTriangle size={16} />}
          title="A new round cannot be started"
          data-testid="round-blocked"
        >
          <Text size="sm">{seed.blocked_reason ?? 'This issue is not ready for a new round.'}</Text>
        </Alert>
      )}

      {/* The open round is incomplete: the one thing that *can* be done here. */}
      {repair?.needs_repair && (
        <Alert
          color="yellow"
          icon={<IconAlertTriangle size={16} />}
          title={`${repair.round_name} is incomplete`}
          data-testid="round-repair-available"
        >
          <Text size="sm">
            {repair.round_name} exists, but{' '}
            {[
              repair.reopen && 'the issue was left closed',
              repair.body_marker && 'the QC Round block in the issue body is out of date',
            ]
              .filter(Boolean)
              .join(' and ')}
            . Repairing re-runs only what is still incomplete; it never posts another round comment,
            and never notifies anyone.
          </Text>
          <RepairAction
            issueNumber={issueNumber}
            label={`Repair ${repair.round_name}`}
            testId="repair-round-submit"
            notification="none"
          />
        </Alert>
      )}

      {noPriorChecklist && (
        <Alert color="blue" icon={<IconInfoCircle size={16} />} data-testid="no-prior-checklist">
          <Text size="sm">
            No prior checklist was found — write the one this round should be reviewed against.
          </Text>
        </Alert>
      )}

      {/*
        Two tabs, matching the create-issue modal's one-concern-per-tab idiom. It
        also collapses the form to a single panel, and it puts the source picker in
        the same row as the name it fills in — the link that was missing when the
        picker floated above as a third unrelated field.

        The read-only context above and the actions below stay outside the tabs, so
        what the round *is* and how to commit it are never a tab away.
      */}
      <Tabs keepMounted={false} value={tab} onChange={(value) => setTab((value as FormTab | null) ?? 'checklist')}>
        <Tabs.List grow>
          <Tabs.Tab
            value="checklist"
            // The only required field lives here, so an empty one is flagged on the
            // tab itself rather than only on a panel the user may not be looking at.
            rightSection={
              checklistContent.trim() === '' ? (
                <Text span c="red" size="sm" data-testid="checklist-tab-required">*</Text>
              ) : null
            }
          >
            Checklist
          </Tabs.Tab>
          <Tabs.Tab value="changes">Changes</Tabs.Tab>
          <Tabs.Tab value="notification">Notification</Tabs.Tab>
        </Tabs.List>

        <Tabs.Panel value="checklist" pt="md" data-testid="checklist-panel">
          <Stack gap="sm">
            <Group grow align="flex-end" wrap="nowrap">
              {seed.checklist_options.length > 1 && (
                <Select
                  label="Start from"
                  data={seed.checklist_options.map((option) => ({
                    value: String(option.round),
                    label: option.round_name,
                  }))}
                  value={sourceRound}
                  onChange={selectSourceRound}
                  allowDeselect={false}
                  disabled={!seed.can_start}
                  data-testid="checklist-source-round"
                />
              )}
              <TextInput
                label="Name"
                placeholder="e.g. Code Review"
                value={checklistName}
                onChange={(e) => setChecklistName(e.currentTarget.value)}
                disabled={!seed.can_start}
              />
            </Group>

            <CommentEditor
              placeholder="- [ ] Checklist item"
              value={checklistContent}
              onChange={setChecklistContent}
              minHeight={200}
              monospace
              showPreviewTabs
            />
          </Stack>
        </Tabs.Panel>

        {/*
          What the reviewer is actually being asked to look at: the two ends of the
          round and the diff between them. The note lives here because it is the
          author's answer to that diff — "why this round is being opened".
        */}
        <Tabs.Panel value="changes" pt="md" data-testid="changes-panel">
          <Stack gap="sm">
            <Stack gap={4} data-testid="round-anchor">
              <CommitLine label="Opens at (HEAD)" hash={seed.anchor} />
              <CommitLine label="Compares against" hash={seed.previous_approval} />
            </Stack>

            <RoundDiff file={seed.file} from={seed.previous_approval} to={seed.anchor} />

            <TextInput
              label="Note (optional)"
              placeholder="Why this round is being opened"
              value={note}
              onChange={(e) => setNote(e.currentTarget.value)}
              disabled={!seed.can_start}
            />
          </Stack>
        </Tabs.Panel>

        <Tabs.Panel value="notification" pt="md" data-testid="notification-panel">
          <Stack gap={4}>
            <SegmentedControl
              data-testid="notification-mode"
              value={notification}
              onChange={(v) => setNotification(v as NotificationMode)}
              disabled={!seed.can_start}
              fullWidth
              data={NOTIFICATION_OPTIONS.map((o) => ({ value: o.value, label: o.label }))}
            />
            <Text size="xs" c="dimmed" data-testid="notification-description">
              {selectedOption.description}
            </Text>
            {notification === 'none' && (
              <Alert
                color="orange"
                icon={<IconAlertTriangle size={16} />}
                data-testid="notification-none-warning"
                mt={4}
              >
                <Text size="sm">
                  The reviewer will not be notified. The round opens silently and nobody is told it exists.
                </Text>
              </Alert>
            )}
          </Stack>
        </Tabs.Panel>
      </Tabs>

      {startRound.error && (
        // Boolean, not a narrowing guard: the failure branch still needs the message.
        roundAlreadyOpen ? (
          <Alert
            color="orange"
            icon={<IconAlertTriangle size={16} />}
            title="A round is already open"
            data-testid="round-still-open"
          >
            <Text size="sm">{startRound.error.message}</Text>
            <Text size="xs" mt={4}>
              Approve the open round first — nothing was posted to the issue.
            </Text>
          </Alert>
        ) : (
          <Alert color="red" title="Failed to start round" data-testid="start-round-error">
            <Text size="sm">{startRound.error.message}</Text>
          </Alert>
        )
      )}

      <Group justify="flex-end" pt="xs">
        <Button variant="default" onClick={onClose}>Cancel</Button>
        {seed.can_start && (
          <Button
            data-testid="start-round-submit"
            loading={startRound.isPending}
            disabled={!canSubmit}
            onClick={handleSubmit}
          >
            Start {seed.next_round_name}
          </Button>
        )}
      </Group>
    </Stack>
  )
}

function CommitLine({ label, hash }: { label: string; hash: string | null }) {
  return (
    <Text size="sm">
      <b>{label}:</b>{' '}
      {hash ? (
        <span style={{ fontFamily: 'monospace' }}>{hash.slice(0, 7)}</span>
      ) : (
        <Text span size="sm" c="dimmed">not available</Text>
      )}
    </Text>
  )
}

/**
 * The diff between the round's two ends.
 *
 * Fetched on demand: the tabs above set `keepMounted={false}`, so this mounts only
 * when the Changes tab is opened and a user who never looks costs no request.
 *
 * Nothing here is an error path in the usual sense. A round can legitimately open at
 * the very commit it compares against — which is exactly the state right after an
 * approval — so "no changes" is a normal, expected answer.
 */
function RoundDiff({ file, from, to }: { file: string; from: string | null; to: string | null }) {
  const enabled = from !== null && to !== null
  const query = useQuery({
    queryKey: commitDiffQueryKey(file, from ?? '', to ?? ''),
    queryFn: () => fetchCommitDiff(file, from as string, to as string),
    enabled,
    staleTime: 5 * 60 * 1000,
  })

  if (!enabled) {
    return (
      <Text size="xs" c="dimmed" data-testid="round-diff-unavailable">
        No commit range to compare.
      </Text>
    )
  }
  if (query.isPending) {
    return (
      <Group gap="xs" data-testid="round-diff-loading">
        <Loader size="xs" />
        <Text size="xs" c="dimmed">Loading changes…</Text>
      </Group>
    )
  }
  if (query.error) {
    return (
      <Alert color="red" data-testid="round-diff-error">
        <Text size="sm">{query.error.message}</Text>
      </Alert>
    )
  }
  if (!query.data.diff) {
    return (
      <Text size="xs" c="dimmed" data-testid="round-diff-empty">
        No changes to {file} between these commits.
      </Text>
    )
  }
  return <DiffView diff={query.data.diff} />
}

/** A fenced ```diff block as the backend renders it, minus the fence. */
function stripDiffFence(diff: string): string {
  const trimmed = diff.trim()
  if (!trimmed.startsWith('```')) return trimmed
  const lines = trimmed.split('\n')
  if (lines[lines.length - 1]?.trim() === '```') lines.pop()
  return lines.slice(1).join('\n')
}

/**
 * Renders the diff with per-line colouring. Deliberately not a markdown renderer:
 * the payload is one fenced block (or, for spreadsheets, a table the same colouring
 * leaves alone), and a `pre` that scrolls on both axes keeps long lines from forcing
 * the modal wider.
 */
function DiffView({ diff }: { diff: string }) {
  const lines = stripDiffFence(diff).split('\n')
  return (
    <div
      data-testid="round-diff"
      style={{
        border: '1px solid var(--mantine-color-gray-3)',
        borderRadius: 6,
        maxHeight: 300,
        overflow: 'auto',
        background: 'var(--mantine-color-gray-0)',
      }}
    >
      <pre
        style={{
          margin: 0,
          padding: '8px 10px',
          fontSize: 12,
          lineHeight: 1.5,
          fontFamily: 'monospace',
        }}
      >
        {lines.map((line, index) => {
          const added = line.startsWith('+')
          const removed = line.startsWith('-')
          const meta = line.startsWith('@@')
          return (
            <div
              key={index}
              style={{
                color: added
                  ? 'var(--mantine-color-green-9)'
                  : removed
                  ? 'var(--mantine-color-red-9)'
                  : meta
                  ? 'var(--mantine-color-blue-7)'
                  : undefined,
                background: added
                  ? 'var(--mantine-color-green-0)'
                  : removed
                  ? 'var(--mantine-color-red-0)'
                  : undefined,
                whiteSpace: 'pre',
              }}
            >
              {line === '' ? ' ' : line}
            </div>
          )
        })}
      </pre>
    </div>
  )
}

const STEP_LABELS: { key: keyof Pick<StartRoundResponse, 'reopened' | 'body_marker' | 'notification'>; label: string }[] = [
  { key: 'reopened', label: 'Set the issue back to open' },
  { key: 'body_marker', label: 'Refresh the QC Round block in the issue body' },
  { key: 'notification', label: 'Post the QC Notification comment' },
]

/**
 * The result of a 201. Always framed as a success: the round comment posted, so
 * the round exists on GitHub. `needs_repair` only means a later, independently
 * retryable step did not land.
 */
function StartRoundResultPanel({
  issueNumber,
  result,
  notificationMode,
  onClose,
}: {
  issueNumber: number
  result: StartRoundResponse
  /** The mode the start attempted, so a failed notification is retried as asked. */
  notificationMode: NotificationMode
  onClose: () => void
}) {
  return (
    <Stack gap="md" data-testid="start-round-result">
      <Alert color="green" title={`${result.round_name} started`} data-testid="start-round-success">
        <Text size="sm">
          The round comment was posted, so {result.round_name} now exists on GitHub, opened at{' '}
          <span style={{ fontFamily: 'monospace' }}>{result.anchor.slice(0, 7)}</span>.
        </Text>
        <Text size="sm" mt={4}>
          <Anchor href={result.round_comment_url} target="_blank" data-testid="round-comment-link">
            View the round comment
          </Anchor>
        </Text>
      </Alert>

      {result.needs_repair && (
        <Alert
          color="yellow"
          icon={<IconAlertTriangle size={16} />}
          title="Some follow-up steps need a retry"
          data-testid="needs-repair"
        >
          <Text size="sm">
            The round itself is fine — only the steps marked <b>Failed</b> below did not land.
            Starting the round again would <b>not</b> retry them: {result.round_name} is open now, so
            another new-round comment would extend it instead of repairing anything.
          </Text>
          <Text size="sm" mt={4}>
            Use <b>Retry follow-up steps</b> below, which re-runs only what is still incomplete. That
            is safe to repeat as often as you like — every step is idempotent.
          </Text>
        </Alert>
      )}

      <Stack gap={6}>
        <Text size="sm" fw={700}>Follow-up steps</Text>
        {STEP_LABELS.map(({ key, label }) => (
          <StepRow key={key} testId={`step-${key}`} label={label} outcome={result[key]} />
        ))}
      </Stack>

      {result.needs_repair && (
        <RepairAction
          issueNumber={issueNumber}
          label="Retry follow-up steps"
          testId="retry-follow-up-steps"
          // A notification is only re-requested when this start actually tried to
          // send one and failed. Otherwise `none`: a round opened deliberately
          // without notifying must never grow a notification out of a repair.
          notification={result.notification.status === 'failed' ? notificationMode : 'none'}
        />
      )}

      <ImpactPreview impacted={result.impacted_issues} />

      <Group justify="flex-end" pt="xs">
        <Button onClick={onClose} data-testid="start-round-done">Done</Button>
      </Group>
    </Stack>
  )
}

/**
 * The repair action: a button, and whatever the repair endpoint said.
 *
 * Shared by the two places a repair is offered — after a round start whose steps
 * did not all land, and on an issue whose open round is already known to be
 * incomplete. A 200 is always a success, including one that reports a step which
 * still failed; only the 409 "nothing to repair" precondition and a real transport
 * failure render as alerts.
 */
function RepairAction({
  issueNumber,
  label,
  testId,
  notification,
}: {
  issueNumber: number
  label: string
  testId: string
  notification: NotificationMode
}) {
  const repair = useRepairRound(issueNumber)
  // Boolean, not a narrowing guard: the other branch still needs the message.
  const nothingToRepair: boolean = repair.error !== null && isNothingToRepairError(repair.error)

  return (
    <Stack gap="sm">
      {repair.data && <RepairResultPanel result={repair.data} />}

      {repair.error &&
        (nothingToRepair ? (
          <Alert
            color="blue"
            icon={<IconInfoCircle size={16} />}
            title="Nothing to repair"
            data-testid="nothing-to-repair"
          >
            <Text size="sm">{repair.error.message}</Text>
          </Alert>
        ) : (
          <Alert color="red" title="Repair failed" data-testid="repair-error">
            <Text size="sm">{repair.error.message}</Text>
          </Alert>
        ))}

      <Group justify="flex-start">
        <Button
          variant="light"
          color="yellow"
          data-testid={testId}
          loading={repair.isPending}
          onClick={() => repair.mutate({ notification })}
        >
          {repair.data || repair.error ? `${label} again` : label}
        </Button>
      </Group>
    </Stack>
  )
}

const REPAIR_STEP_LABELS: {
  key: keyof Pick<RepairRoundResponse, 'reopened' | 'body_marker' | 'notification'>
  label: string
}[] = [
  { key: 'reopened', label: 'Set the issue back to open' },
  { key: 'body_marker', label: 'Refresh the QC Round block in the issue body' },
  { key: 'notification', label: 'Post the QC Notification comment' },
]

/**
 * The result of a repair. Like the start, always framed as an outcome rather than
 * an error: a step that still failed is reported per step, and `Skipped` here means
 * the step was already correct or was not requested — not that something went wrong.
 */
function RepairResultPanel({ result }: { result: RepairRoundResponse }) {
  return (
    <Stack gap={6} data-testid="repair-result">
      <Alert
        color={result.needs_repair ? 'yellow' : 'green'}
        icon={result.needs_repair ? <IconAlertTriangle size={16} /> : undefined}
        title={result.needs_repair ? 'Some steps still failed' : 'Follow-up steps completed'}
        data-testid={result.needs_repair ? 'repair-still-failing' : 'repair-success'}
      >
        <Text size="sm">
          {result.needs_repair
            ? `The step(s) marked Failed below still did not land. ${result.round_name} itself is unaffected, and this retry can be repeated once the cause is fixed.`
            : result.repaired
            ? `${result.round_name} is now fully recorded.`
            : `Nothing needed repairing — ${result.round_name} was already complete.`}
        </Text>
      </Alert>
      {REPAIR_STEP_LABELS.map(({ key, label }) => (
        <StepRow
          key={key}
          testId={`repair-step-${key}`}
          label={label}
          outcome={result[key]}
          skippedHint="Already correct, or not requested"
        />
      ))}
    </Stack>
  )
}

const STEP_PRESENTATION: Record<StepOutcome['status'], { color: string; text: string; hint: string }> = {
  done: { color: 'green', text: 'Done', hint: '' },
  skipped: { color: 'gray', text: 'Skipped', hint: 'Deliberately not run' },
  failed: { color: 'red', text: 'Failed', hint: 'Something went wrong — safe to retry' },
}

function StepRow({
  testId,
  label,
  outcome,
  skippedHint,
}: {
  testId: string
  label: string
  outcome: StepOutcome
  /** Overrides the `skipped` hint: on the repair path it means "nothing to do". */
  skippedHint?: string
}) {
  const base = STEP_PRESENTATION[outcome.status]
  const presentation =
    outcome.status === 'skipped' && skippedHint !== undefined
      ? { ...base, hint: skippedHint }
      : base
  return (
    <div data-testid={testId} style={{ display: 'flex', alignItems: 'flex-start', gap: 8 }}>
      <Badge color={presentation.color} variant="light" size="sm" style={{ flexShrink: 0 }}>
        {presentation.text}
      </Badge>
      <div>
        <Text size="sm">{label}</Text>
        {presentation.hint && (
          <Text size="xs" c="dimmed">{presentation.hint}</Text>
        )}
        {outcome.error && (
          <Text size="xs" c="red" style={{ wordBreak: 'break-word' }}>{outcome.error}</Text>
        )}
      </div>
    </div>
  )
}

/** Information only — one layer of downstream issues. Nothing was written to them. */
function ImpactPreview({ impacted }: { impacted: ImpactedIssues }) {
  if (!impacted.api_available) {
    return (
      <Alert color="gray" icon={<IconInfoCircle size={16} />} data-testid="impact-unavailable">
        <Text size="sm">
          Which QCs depend on this file could not be checked — the dependency API was unavailable.
          Nothing was written to any downstream issue either way, and the previous approval still
          stands.
        </Text>
      </Alert>
    )
  }

  if (impacted.issues.length === 0) {
    return (
      <Text size="sm" c="dimmed" data-testid="impact-empty">
        No downstream QCs appear to depend on this file.
      </Text>
    )
  }

  return (
    <Stack gap={6} data-testid="impact-list">
      <Text size="sm" fw={700}>Downstream QCs that depend on this file</Text>
      <Text size="xs" c="dimmed">
        Notice only — the previous approval still stands, so these QCs remain valid. Nothing was
        written to them and no action was taken on them.
      </Text>
      {impacted.issues.map((item) => (
        <Text key={item.issue_number} size="sm" data-testid={`impact-issue-${item.issue_number}`}>
          <b>#{item.issue_number}</b> {item.file_name}
          <Text span size="xs" c="dimmed"> · {item.milestone} · {item.relationship}</Text>
        </Text>
      ))}
    </Stack>
  )
}
