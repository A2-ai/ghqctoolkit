import { useState } from 'react'
import {
  Alert,
  Anchor,
  Badge,
  Button,
  Group,
  Loader,
  Modal,
  Radio,
  Select,
  Stack,
  Tabs,
  Text,
  Textarea,
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
  type GapContinuity,
  type RoundSeedResponse,
  type StartRoundResponse,
  type StepOutcome,
} from '~/api/rounds'
import { useQuery } from '@tanstack/react-query'
import { commitDiffQueryKey, fetchCommitDiff } from '~/api/commits'
import { fetchCommentPreview } from '~/api/preview'
import { wrapInGithubStyles } from '~/utils/github'
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
  /**
   * The branch the previous approval was reviewed on, for the divergence note.
   *
   * `GapContinuity` carries no `previous_branch` and needs none: every Round declares
   * a branch (D5), so the caller reads it off the last closed Round segment of the
   * issue status it already holds (see the contract's §7.6). Passed in rather than
   * fetched, so this modal still needs exactly one request. Omitted → the note drops
   * the branch label, whose load-bearing content is the merge-base anyway.
   */
  previousBranch?: string | null
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
export function StartRoundModal({ issueNumber, issueTitle, issueUrl, repair, previousBranch, onClose }: StartRoundModalProps) {
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
          previousBranch={previousBranch}
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
  previousBranch,
  onClose,
}: {
  issueNumber: number
  issueTitle?: string
  issueUrl?: string
  repair?: RoundRepairStatus | null
  previousBranch?: string | null
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
      previousBranch={previousBranch}
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
  previousBranch,
  onClose,
}: {
  issueNumber: number
  issueTitle?: string
  issueUrl?: string
  seed: RoundSeedResponse
  repair?: RoundRepairStatus | null
  previousBranch?: string | null
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
  // Distinct from `note`: that one records why the round exists, this one is
  // addressed to the reviewer who is about to be @-mentioned.
  const [notificationNote, setNotificationNote] = useState('')
  // Notification preview, in the same shape as the notify tab's: a nested modal
  // holding the rendered comment. Plain state rather than a query because it is
  // fired by a button and reflects unsaved form input, not server state.
  const [previewOpen, setPreviewOpen] = useState(false)
  const [previewHtml, setPreviewHtml] = useState<string | null>(null)
  const [previewLoading, setPreviewLoading] = useState(false)
  const [tab, setTab] = useState<FormTab>('changes')
  const startRound = useStartRound(issueNumber)

  // A 201 is a success even when it reports failed steps — see StartRoundResultPanel.
  if (startRound.data) {
    return (
      <StartRoundResultPanel
        issueNumber={issueNumber}
        result={startRound.data}
        notificationMode={notification}
        notificationNote={notificationNote}
        onClose={onClose}
      />
    )
  }

  const noPriorChecklist = seed.checklist_content === null
  const canSubmit = seed.can_start && checklistContent.trim().length > 0
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

  /**
   * Preview the notification comment this round would post.
   *
   * Reuses the notify tab's endpoint rather than adding a round-specific one: the
   * round's notification *is* a `QCComment` built from the same four inputs, so the
   * preview is rendered by the very code that will post it.
   */
  async function handlePreview() {
    if (seed.anchor === null) return
    setPreviewLoading(true)
    try {
      const html = await fetchCommentPreview(issueNumber, {
        current_commit: seed.anchor,
        previous_commit: seed.previous_approval,
        note: notificationNote.trim() === '' ? null : notificationNote.trim(),
        include_diff: notification === 'full',
      })
      setPreviewHtml(html)
    } catch (error) {
      setPreviewHtml(`<pre>Error: ${(error as Error).message}</pre>`)
    } finally {
      setPreviewLoading(false)
      setPreviewOpen(true)
    }
  }

  function handleSubmit() {
    startRound.mutate({
      checklist_content: checklistContent,
      checklist_name: checklistName.trim() === '' ? null : checklistName.trim(),
      note: note.trim() === '' ? null : note.trim(),
      // Sent independently of `note`: the API has no fallback between the two, so
      // an empty message means the notification carries none.
      notification_note: notificationNote.trim() === '' ? null : notificationNote.trim(),
      notification,
    })
  }

  return (
    <>
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
      <Tabs keepMounted={false} value={tab} onChange={(value) => setTab((value as FormTab | null) ?? 'changes')}>
        <Tabs.List grow>
          {/* Changes leads: what moved is the reason the round exists. */}
          <Tabs.Tab value="changes">Changes</Tabs.Tab>
          <Tabs.Tab
            value="checklist"
            // The only required field lives here, and this is no longer the tab the
            // modal opens on — so an empty one is flagged on the tab itself rather
            // than only inside a panel the user may not have visited.
            rightSection={
              checklistContent.trim() === '' ? (
                <Text span c="red" size="sm" data-testid="checklist-tab-required">*</Text>
              ) : null
            }
          >
            Checklist
          </Tabs.Tab>
          <Tabs.Tab value="notification">Notification</Tabs.Tab>
        </Tabs.List>

        {/*
          What the reviewer is actually being asked to look at: the two ends of the
          round and the diff between them. The note lives here because it is the
          author's answer to that diff — "why this round is being opened".
        */}
        <Tabs.Panel value="changes" pt="md" data-testid="changes-panel">
          <Stack gap="sm">
            <Stack gap={4} data-testid="round-anchor">
              {/*
                The branch leads: a round is QC'd where the work is now, which need not
                be where the issue was created, and it changes what the two commits
                below even mean.
              */}
              <Text size="sm" data-testid="round-branch">
                <b>Branch:</b>{' '}
                {seed.branch ? (
                  <span style={{ fontFamily: 'monospace' }}>{seed.branch}</span>
                ) : (
                  <Text span size="sm" c="dimmed">not available</Text>
                )}
              </Text>
              <CommitLine label="Opens at (HEAD)" hash={seed.anchor} />
              {/*
                Labelled by what it *is*, which is only the previous approval when that
                approval is reachable from this branch — see the divergence note below.
              */}
              <CommitLine
                label={seed.divergence ? 'Compares against (merge-base)' : 'Compares against'}
                hash={seed.comparison_base ?? seed.previous_approval}
              />
            </Stack>

            {seed.divergence && (
              <DivergenceNote
                divergence={seed.divergence}
                branch={seed.branch}
                previousBranch={previousBranch ?? null}
              />
            )}

            {/*
              Diffed from the comparison base, not the approval: on a divergent branch
              the approval is not an ancestor, so a diff against it would describe
              changes that are not this round's.

              Withheld entirely when the two ends share no history (M4 `Unrelated`):
              the note directly above says no diff between them is meaningful, and
              rendering one under that sentence would contradict it. There is no
              base to compare from, so there is nothing to show.
            */}
            {seed.divergence?.kind !== 'unrelated' && (
              <RoundDiff
                file={seed.file}
                from={seed.comparison_base ?? seed.previous_approval}
                to={seed.anchor}
              />
            )}

            <TextInput
              label="Note (optional)"
              placeholder="Why this round is being opened"
              value={note}
              onChange={(e) => setNote(e.currentTarget.value)}
              disabled={!seed.can_start}
            />
          </Stack>
        </Tabs.Panel>

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
          Radio cards rather than a segmented control. A segmented control is a
          view-switcher: it suits options that need no explanation and a row too
          cramped to give them one, which is what this was before it had a tab. Here
          each mode needs a sentence, and showing all three at once beats a single
          line that swaps as you click. The cards also give the silent-mode warning
          somewhere to sit inside the option it belongs to.
        */}
        <Tabs.Panel value="notification" pt="md" data-testid="notification-panel">
          <Stack gap="sm">
            <Radio.Group
              value={notification}
              onChange={(v) => setNotification(v as NotificationMode)}
              data-testid="notification-mode"
            >
              <Stack gap={6}>
                {NOTIFICATION_OPTIONS.map((option) => (
                  <Radio.Card
                    key={option.value}
                    value={option.value}
                    p="xs"
                    // On the card as well as the indicator: the whole card is the
                    // hit target, so an indicator-only guard leaves a blocked form
                    // still switching modes.
                    disabled={!seed.can_start}
                    data-testid={`notification-mode-${option.value}`}
                    style={{ cursor: seed.can_start ? 'pointer' : 'not-allowed' }}
                  >
                    <Group gap="sm" wrap="nowrap" align="flex-start">
                      <Radio.Indicator disabled={!seed.can_start} mt={2} />
                      <Stack gap={2}>
                        <Text size="sm" fw={600}>{option.label}</Text>
                        <Text size="xs" c="dimmed">{option.description}</Text>
                        {option.value === 'none' && notification === 'none' && (
                          <Group gap={4} wrap="nowrap" data-testid="notification-none-warning">
                            <IconAlertTriangle size={14} color="var(--mantine-color-orange-7)" />
                            <Text size="xs" c="orange.7">
                              Nobody is told the round exists.
                            </Text>
                          </Group>
                        )}
                      </Stack>
                    </Group>
                  </Radio.Card>
                ))}
              </Stack>
            </Radio.Group>

            {/*
              The reviewer-facing message, kept with the comment that carries it
              rather than with the round's own note on the Changes tab. Hidden when
              nothing will be posted: a message with no comment to ride on is a field
              that silently discards what you type.
            */}
            {notification !== 'none' && (
              <Textarea
                label="Message to the reviewer (optional)"
                description="Added to the QC Notification comment, above the commit metadata."
                placeholder="Anything they should know before reviewing"
                value={notificationNote}
                onChange={(e) => setNotificationNote(e.currentTarget.value)}
                disabled={!seed.can_start}
                autosize
                minRows={2}
                maxRows={6}
                data-testid="notification-note"
              />
            )}

            {/*
              Only offered when something will actually be posted: there is no comment
              to preview in silent mode. The anchor guard mirrors the submit button's —
              without it there is no commit to render against.
            */}
            {notification !== 'none' && (
              <Group justify="flex-end">
                <Button
                  variant="default"
                  size="xs"
                  loading={previewLoading}
                  disabled={seed.anchor === null}
                  onClick={handlePreview}
                  data-testid="notification-preview"
                >
                  Preview notification
                </Button>
              </Group>
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

    {/* Rendered by the same builder that will post it — see handlePreview. */}
    <Modal
      opened={previewOpen}
      onClose={() => setPreviewOpen(false)}
      title="Notification Preview"
      size={800}
      centered
      styles={{ header: { paddingTop: 12, paddingBottom: 12 }, body: { paddingBottom: 20 } }}
    >
      <iframe
        srcDoc={previewHtml ? wrapInGithubStyles(previewHtml) : ''}
        style={{ width: '100%', height: 450, border: '1px solid var(--mantine-color-gray-3)', borderRadius: 6 }}
        title="Notification Preview"
        data-testid="notification-preview-frame"
      />
    </Modal>
    </>
  )
}

/**
 * Why the comparison is not the previous approval.
 *
 * Phrased as a fact about git rather than a warning about the round: opening a round on
 * a branch that does not contain the last approval is legitimate — the round still
 * opens — but the diff means something different, and that has to be said plainly.
 */
function DivergenceNote({
  divergence,
  branch,
  previousBranch,
}: {
  divergence: GapContinuity
  branch: string | null
  /** Joined in by the caller from the last closed Round segment; see §7.6. */
  previousBranch: string | null
}) {
  const previous = previousBranch
  const here = branch ?? 'this branch'
  return (
    <Alert
      color="orange"
      icon={<IconAlertTriangle size={16} />}
      data-testid="round-divergence"
      p="xs"
    >
      <Text size="xs">
        {divergence.kind === 'diverged' ? (
          <>
            The previous approval{previous ? <> (on <b>{previous}</b>)</> : null} is not part of{' '}
            <b>{here}</b>, so the comparison uses the last commit the two branches share.
            Changes made on{previous ? <> <b>{previous}</b></> : <> the other branch</>} after that
            point are included in the diff below.
          </>
        ) : (
          <>
            The previous approval{previous ? <> (on <b>{previous}</b>)</> : null} shares no history
            with <b>{here}</b>. No diff between them is meaningful — review the file directly.
          </>
        )}
      </Text>
    </Alert>
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
 * The tabs set `keepMounted={false}`, so this unmounts whenever another tab is
 * showing. React Query's cache is what makes that cheap — the diff is fetched once
 * per commit range, not once per visit to this tab.
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
  notificationNote,
  onClose,
}: {
  issueNumber: number
  result: StartRoundResponse
  /** The mode the start attempted, so a failed notification is retried as asked. */
  notificationMode: NotificationMode
  /** The message it attempted to send, so the retry is not silently emptier. */
  notificationNote: string
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
          notificationNote={notificationNote.trim() === '' ? undefined : notificationNote.trim()}
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
  notificationNote,
}: {
  issueNumber: number
  label: string
  testId: string
  notification: NotificationMode
  /**
   * Message for the reviewer, when this repair is retrying a notification the user
   * had already written one for. It exists nowhere but the comment that failed to
   * post, so re-sending it is the only way it survives. Omitted → the backend falls
   * back to the round's own note.
   */
  notificationNote?: string
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
          onClick={() => repair.mutate({ notification, notification_note: notificationNote ?? null })}
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
  // A skip the server explained beats any static hint. `skipped_reason` is present only
  // when the step could not be *attempted* — a repair's notification on a round that
  // could not be placed, or its body marker when the round comment URL is unknown — so
  // "Already correct, or not requested" would be actively wrong there. The wording comes
  // from `UnplaceableReason::describe()`, the same string the CLI prints.
  const skippedText =
    outcome.status === 'skipped'
      ? (outcome.skipped_reason ?? skippedHint)
      : undefined
  const presentation =
    skippedText !== undefined ? { ...base, hint: skippedText } : base
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
