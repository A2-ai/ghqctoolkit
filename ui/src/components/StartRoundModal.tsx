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
  Stack,
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
import { CommentEditor } from './CommentEditor'

export interface StartRoundModalProps {
  /** The issue to start a new QC round for; `null` keeps the modal closed. */
  issueNumber: number | null
  /** File name of the QC'd file, shown as context in the header. Optional. */
  issueTitle?: string
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
export function StartRoundModal({ issueNumber, issueTitle, repair, onClose }: StartRoundModalProps) {
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
  repair,
  onClose,
}: {
  issueNumber: number
  issueTitle?: string
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
      seed={seedQuery.data}
      repair={repair}
      onClose={onClose}
    />
  )
}

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
  seed,
  repair,
  onClose,
}: {
  issueNumber: number
  issueTitle?: string
  seed: RoundSeedResponse
  repair?: RoundRepairStatus | null
  onClose: () => void
}) {
  const [checklistContent, setChecklistContent] = useState(seed.checklist_content ?? '')
  const [checklistName, setChecklistName] = useState(seed.checklist_name ?? '')
  const [note, setNote] = useState('')
  const [notification, setNotification] = useState<NotificationMode>('full')
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
        {issueTitle && (
          <Text size="xs" c="dimmed">#{issueNumber} · {issueTitle}</Text>
        )}
      </Stack>

      {/* Target commit — read-only by design: the anchor is always branch HEAD. */}
      <Stack gap={4} data-testid="round-anchor">
        <CommitLine label="Target commit (HEAD)" hash={seed.anchor} />
        <CommitLine label="Compared against" hash={seed.previous_approval} />
        <Text size="xs" c="dimmed">
          A round always opens at the current HEAD of the issue's branch, so there is nothing to choose here.
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
            No prior checklist was found for this issue, so the editor below starts empty. Write the
            checklist this round should be reviewed against.
          </Text>
        </Alert>
      )}

      <TextInput
        label="Checklist name"
        placeholder="e.g. Code Review"
        description="Recorded in the round comment as the audit record of which template this round used. Clear it to record none."
        value={checklistName}
        onChange={(e) => setChecklistName(e.currentTarget.value)}
        disabled={!seed.can_start}
      />

      <CommentEditor
        label="Checklist"
        placeholder="- [ ] Checklist item"
        value={checklistContent}
        onChange={setChecklistContent}
        minHeight={180}
        monospace
        showPreviewTabs
        required
      />
      <Text size="xs" c="dimmed" mt={-8}>
        The full checklist for this round — add, reword, reorder or delete items freely.
      </Text>

      <TextInput
        label="Note (optional)"
        placeholder="Why this round is being opened"
        description="Kept to a single line: the round metadata records one line only."
        value={note}
        onChange={(e) => setNote(e.currentTarget.value)}
        disabled={!seed.can_start}
      />

      <Stack gap={4}>
        <Text size="sm" fw={500}>Notification</Text>
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

const STEP_LABELS: { key: keyof Pick<StartRoundResponse, 'reopened' | 'body_marker' | 'notification'>; label: string }[] = [
  { key: 'reopened', label: 'Reopen the issue' },
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
  { key: 'reopened', label: 'Reopen the issue' },
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
          Downstream impact could not be checked — the dependency API was unavailable. There may or
          may not be issues affected by this round.
        </Text>
      </Alert>
    )
  }

  if (impacted.issues.length === 0) {
    return (
      <Text size="sm" c="dimmed" data-testid="impact-empty">
        No downstream issues appear to be affected by this round.
      </Text>
    )
  }

  return (
    <Stack gap={6} data-testid="impact-list">
      <Text size="sm" fw={700}>Downstream issues that may be affected</Text>
      <Text size="xs" c="dimmed">
        Information only — nothing was written to these issues, and no action was taken on them.
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
