import { useEffect, useMemo, useState } from 'react'
import {
  Alert,
  Anchor,
  Badge,
  Button,
  Checkbox,
  Group,
  Loader,
  Modal,
  Select,
  Stack,
  Tabs,
  Text,
  TextInput,
  Tooltip,
} from '@mantine/core'
import { useDebouncedValue } from '@mantine/hooks'
import { useQueryClient } from '@tanstack/react-query'
import type {
  CreateCommentRequest,
  CreateRoundResponse,
  IssueStatusResponse,
  RoundInfo,
} from '~/api/issues'
import { latestRound, postRound, roundApprovedCommit } from '~/api/issues'
import type { RoundDiffPreviewRequest, RoundPreviewRequest } from '~/api/preview'
import { useCommentPreview, useRoundDiffPreview, useRoundPreview } from '~/api/preview'
import { useRepoInfo } from '~/api/repo'
import { wrapInGithubStyles } from '~/utils/github'
import { CommentEditor } from './CommentEditor'
import { InheritedBranchBadge, NoCohesiveHistoryBadge } from './RoundBadges'

interface Props {
  opened: boolean
  onClose: () => void
  status: IssueStatusResponse
}

/**
 * Resets every checked item of a base round's checklist (D37: `checklist_content`
 * already excludes its `# ` heading, so nothing here re-adds one — re-adding it
 * would emit the heading twice and make the *next* round's "second H1" parse latch
 * onto the wrong heading).
 */
export function resetCheckboxes(content: string): string {
  return content.replace(/^(\s*[-*+]\s*)\[[xX]\]/gm, '$1[ ]')
}

type PreviewTab = 'round' | 'notification'

/**
 * Round 1 is the initial QC: it has no `# QC Round 1` comment because the issue body
 * *is* round 1 (D2). Naming it that way is what tells a user seeding round 2 what the
 * checklist in front of them came from.
 */
function roundLabel(index: number): string {
  return index === 1 ? 'Round 1 (the initial QC)' : `Round ${index}`
}

/**
 * U2: tabbed, no scrolling. The branch and the start commit are read-only from the
 * checkout (D23) — there is deliberately no commit picker anywhere in the round flow.
 */
export function NewRoundModal({ opened, onClose, status }: Props) {
  const { issue, rounds, drift } = status
  const { data: repoInfo } = useRepoInfo()
  const queryClient = useQueryClient()

  const priorRound = latestRound(status)
  const nextRoundIndex = priorRound.index + 1
  const priorApprovalCommit = roundApprovedCommit(priorRound)

  const [activeTab, setActiveTab] = useState<string | null>('round')
  const [baseRoundIndex, setBaseRoundIndex] = useState(priorRound.index)
  const [checklistName, setChecklistName] = useState(priorRound.checklist_name)
  const [checklistContent, setChecklistContent] = useState(() => resetCheckboxes(priorRound.checklist_content))
  const [notify, setNotify] = useState(true)
  const [note, setNote] = useState('')
  const [includeDiff, setIncludeDiff] = useState(true)
  const [posting, setPosting] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [result, setResult] = useState<CreateRoundResponse | null>(null)
  const [previewOpen, setPreviewOpen] = useState(false)
  const [previewTab, setPreviewTab] = useState<PreviewTab>('round')

  const baseRound: RoundInfo = useMemo(
    () => rounds.find((r) => r.index === baseRoundIndex) ?? priorRound,
    [rounds, baseRoundIndex, priorRound],
  )

  // Reset when the modal is (re)opened, or the issue behind it changes.
  useEffect(() => {
    if (!opened) return
    setActiveTab('round')
    setBaseRoundIndex(priorRound.index)
    setChecklistName(priorRound.checklist_name)
    setChecklistContent(resetCheckboxes(priorRound.checklist_content))
    setNotify(true)
    setNote('')
    setIncludeDiff(true)
    setError(null)
    setResult(null)
    setPreviewOpen(false)
    setPreviewTab('round')
  }, [opened, issue.number]) // eslint-disable-line react-hooks/exhaustive-deps

  function handleBaseRoundChange(value: string | null) {
    if (value === null) return
    const index = Number(value)
    const round = rounds.find((r) => r.index === index)
    if (!round) return
    setBaseRoundIndex(index)
    setChecklistName(round.checklist_name)
    setChecklistContent(resetCheckboxes(round.checklist_content))
  }

  const startCommit = repoInfo?.local_commit ?? ''
  const branch = repoInfo?.branch ?? ''

  // U3: nothing to diff when the checkout is still sitting on the prior approval —
  // the empty-drift case.
  const noDrift = !!priorApprovalCommit && !!startCommit && priorApprovalCommit === startCommit
  const effectiveNotify = notify && !noDrift

  const canCreate =
    !!startCommit &&
    !!branch &&
    checklistName.trim().length > 0 &&
    checklistContent.trim().length > 0

  // The Notification tab only exists while a notification is actually going to be
  // posted, so the selected tab is *derived* rather than stored — turning notify off
  // while looking at it must not leave a tab selected that is no longer rendered.
  const activePreviewTab: PreviewTab = effectiveNotify ? previewTab : 'round'

  // D47: the preview is rendered server-side by the same `QCRound::generate_body`
  // the creation posts, so it cannot drift from what lands on the issue — and only
  // the server can emit the `[file contents at initial qc commit]` link. The round
  // index is not sent: the server derives it exactly as `POST /rounds` does.
  //
  // Gated on the preview modal being open and on debounced editable fields — the
  // checklist and the note are textareas and this must not be a request per keystroke.
  const [debouncedChecklistName] = useDebouncedValue(checklistName, 400)
  const [debouncedChecklistContent] = useDebouncedValue(checklistContent, 400)
  const [debouncedNote] = useDebouncedValue(note, 400)

  const previewRequest = useMemo<RoundPreviewRequest | null>(() => {
    if (!previewOpen || activePreviewTab !== 'round' || !startCommit || !branch) return null
    return {
      issue_number: issue.number,
      start_commit: startCommit,
      branch,
      checklist: { name: debouncedChecklistName.trim(), content: debouncedChecklistContent },
    }
  }, [previewOpen, activePreviewTab, startCommit, branch, issue.number, debouncedChecklistName, debouncedChecklistContent])
  const roundPreview = useRoundPreview(previewRequest)

  // The notification is a `QCComment` whose commit pair `create_round` fixes for us
  // (D5): current = this round's start, previous = the prior round's approval. Sending
  // any other pair would preview a comment the creation would not post.
  const notificationRequest = useMemo<CreateCommentRequest | null>(() => {
    if (!previewOpen || activePreviewTab !== 'notification' || !startCommit) return null
    return {
      current_commit: startCommit,
      previous_commit: priorApprovalCommit ?? null,
      note: debouncedNote.trim() || null,
      include_diff: includeDiff,
    }
  }, [previewOpen, activePreviewTab, startCommit, priorApprovalCommit, debouncedNote, includeDiff])
  const notificationPreview = useCommentPreview(issue.number, notificationRequest)

  // The Round tab's substance: what actually changed in this file since the approval
  // the next round would start from. The old end of the pair is the server's to choose
  // (D5), so only the checkout commit is sent — and nothing is asked at all when the
  // checkout *is* the approval, because U3 already proves there is no difference.
  const diffRequest = useMemo<RoundDiffPreviewRequest | null>(() => {
    if (activeTab !== 'round' || !startCommit || noDrift || !priorApprovalCommit) return null
    return { issue_number: issue.number, start_commit: startCommit }
  }, [activeTab, startCommit, noDrift, priorApprovalCommit, issue.number])
  const diffPreview = useRoundDiffPreview(diffRequest)

  // Counts only — which commit is which is the server's call (U7). `drift` is the
  // trailing gap after the latest round, so it is exactly "since the approval".
  const driftCommitCount = drift.commits.length
  const driftFileChangeCount = drift.commits.filter((commit) => commit.file_changed).length

  function openPreview(tab: PreviewTab) {
    setPreviewTab(tab)
    setPreviewOpen(true)
  }

  // U6: a divergent preceding gap anywhere in the thread means the rounds share no
  // cohesive history; a divergent drift means the approval this round starts from is
  // not in the branch's ancestry (D31).
  const divergentGap = rounds.some((r) => r.preceding_gap.divergent)

  async function handleCreate() {
    setPosting(true)
    setError(null)
    try {
      const response = await postRound(issue.number, {
        start_commit: startCommit,
        branch,
        checklist: { name: checklistName.trim(), content: checklistContent },
        notify: effectiveNotify,
        note: note.trim() || null,
        include_diff: includeDiff,
      })
      setResult(response)
      void queryClient.invalidateQueries({ queryKey: ['issue', 'status', issue.number] })
    } catch (err) {
      setError((err as Error).message)
    } finally {
      setPosting(false)
    }
  }

  return (
    <Modal
      opened={opened}
      onClose={onClose}
      title={`Start QC Round ${nextRoundIndex} — ${issue.title}`}
      size={800}
      centered
    >
      {result ? (
        <Stack gap="sm" data-testid="new-round-result">
          <Text size="sm">
            Round {result.round_index} started.{' '}
            <Anchor href={result.comment_url} target="_blank">View the round comment</Anchor>
          </Text>
          {/* D45: the notification's three outcomes are distinct — a failure the user
              can retry manually must not look like "never asked for". */}
          {result.notification.kind === 'posted' && (
            <Text size="sm">
              <Anchor href={result.notification.url} target="_blank">View the notification comment</Anchor>
            </Text>
          )}
          {result.notification.kind === 'failed' && (
            <Alert color="yellow" p="xs" data-testid="new-round-notification-failed">
              <Text size="xs">
                The round comment was posted, but the notification failed: {result.notification.error}.
                Post it manually if the reviewers need it.
              </Text>
            </Alert>
          )}
          {/* D45: re-opening is non-fatal but never silent — a closed issue with an
              unapproved latest round reads as "approval required" (S3). */}
          {!result.reopened && (
            <Alert color="orange" p="xs" data-testid="new-round-not-reopened">
              <Text size="xs">
                Round created, but the issue could not be re-opened — reopen it on GitHub
                or the QC will show as Approval required.
              </Text>
            </Alert>
          )}
          <Group justify="flex-end">
            <Button onClick={onClose}>Close</Button>
          </Group>
        </Stack>
      ) : (
        <>
        <Tabs value={activeTab} onChange={setActiveTab} keepMounted={false}>
          <Tabs.List grow>
            <Tabs.Tab value="round">Round</Tabs.Tab>
            <Tabs.Tab value="checklist">Checklist</Tabs.Tab>
            <Tabs.Tab value="notify">Notify</Tabs.Tab>
          </Tabs.List>

          <Tabs.Panel value="round" pt="md">
            <Stack gap="sm">
              <Group gap="xs">
                <Badge variant="light" color="blue" size="sm">Round {nextRoundIndex}</Badge>
                {/* D56: this modal is one of the surfaces where the prior round is the
                    one being viewed — its branch is what the new round is compared
                    against, so an inherited one must not be silent. */}
                {priorRound.branch_inherited && <InheritedBranchBadge branch={priorRound.branch} />}
                {divergentGap && <NoCohesiveHistoryBadge />}
                {drift.divergent && (
                  <Badge color="red" variant="light" size="xs" data-testid="new-round-drift-divergent">
                    approval commit not in branch history
                  </Badge>
                )}
              </Group>

              {/* D23: both come from the checkout. Rendered as facts rather than as
                  `readOnly` text inputs — a bordered box reads as editable, and the
                  round flow has no commit picker for it to imply. */}
              <CheckoutFacts branch={branch} startCommit={startCommit} />

              {branch && branch !== priorRound.branch && (
                <Alert color="yellow" p="xs">
                  <Text size="xs">
                    Round {priorRound.index} was on <b>{priorRound.branch}</b>; this round will be
                    declared on <b>{branch}</b>.
                  </Text>
                </Alert>
              )}

              {/* D56: branch scopes both the round's and its gap's commit walk
                  (D7/D9), so "round N inherited its branch" is stated, not implied. */}
              {priorRound.branch_inherited && (
                <Alert color="gray" p="xs" data-testid="new-round-inherited-branch">
                  <Text size="xs">
                    Round {priorRound.index} declared no branch of its own — it inherited{' '}
                    <b>{priorRound.branch}</b> from the round before it.
                  </Text>
                </Alert>
              )}

              {/* The question this tab exists to answer: is there anything here worth a
                  round? So it shows the change itself, not just the commit it ends at. */}
              <div>
                <Text size="sm" fw={600}>
                  Changes since round {priorRound.index}'s approval
                </Text>
                {noDrift ? (
                  <Text size="xs" c="dimmed" data-testid="new-round-diff-no-changes">
                    The checkout is the commit round {priorRound.index} was approved at, so
                    this file has not changed.
                  </Text>
                ) : (
                  <>
                    <Text size="xs" c="dimmed" data-testid="new-round-drift-summary">
                      {driftCommitCount} commit{driftCommitCount === 1 ? '' : 's'} since the
                      approval, {driftFileChangeCount} of which touched this file.
                    </Text>
                    <div data-testid="new-round-diff" style={{ marginTop: 6 }}>
                      {diffPreview.isError ? (
                        <Alert color="red" p="xs" data-testid="new-round-diff-error">
                          <Text size="xs">
                            Could not render the change since the approval:{' '}
                            {diffPreview.error.message}
                          </Text>
                        </Alert>
                      ) : diffPreview.data === undefined ? (
                        <Group justify="center" py="md" data-testid="new-round-diff-loading">
                          <Loader size="sm" />
                        </Group>
                      ) : (
                        <iframe
                          srcDoc={wrapInGithubStyles(diffPreview.data)}
                          style={{ width: '100%', height: 300, border: '1px solid var(--mantine-color-gray-3)', borderRadius: 6 }}
                          title="Round Diff Preview"
                        />
                      )}
                    </div>
                  </>
                )}
              </div>
            </Stack>
          </Tabs.Panel>

          <Tabs.Panel value="checklist" pt="md">
            <Stack gap="sm">
              {/* The seed's provenance is always stated, not only when it is
                  selectable: a user creating round 2 has one base round and still
                  needs to know the checklist in front of them is the initial QC's. */}
              {rounds.length > 1 ? (
                <Select
                  label="Base round"
                  description="Seeds the checklist below, with every tick reset"
                  data={rounds.map((r) => ({
                    value: String(r.index),
                    label: `${roundLabel(r.index)}${r.checklist_name ? ` — ${r.checklist_name}` : ''}`,
                  }))}
                  value={String(baseRound.index)}
                  onChange={handleBaseRoundChange}
                  allowDeselect={false}
                  data-testid="new-round-base-round"
                />
              ) : (
                <Text size="sm" c="dimmed" data-testid="new-round-base-round-static">
                  Seeded from <b>{roundLabel(baseRound.index)}</b>
                  {baseRound.checklist_name ? ` — ${baseRound.checklist_name}` : ''}, with every
                  tick reset. It is the only round to seed from.
                </Text>
              )}
              <TextInput
                label="Checklist name"
                value={checklistName}
                onChange={(e) => setChecklistName(e.currentTarget.value)}
                data-testid="new-round-checklist-name"
              />
              {/* D37: the content excludes its `# ` heading — the heading is emitted once,
                  by the checklist's own Display, when the comment is posted. */}
              <CommentEditor
                label="Checklist"
                value={checklistContent}
                onChange={setChecklistContent}
                monospace
                minHeight={220}
              />
            </Stack>
          </Tabs.Panel>

          {/* The notification is a second comment with its own commit pair, note and
              diff toggle, so it gets its own tab rather than sharing the round's. */}
          <Tabs.Panel value="notify" pt="md">
            <Stack gap="sm">
              <Tooltip
                label="The checkout is still on the previous approval commit — there is nothing to diff"
                disabled={!noDrift}
                withArrow
                position="right"
                multiline
                w={280}
              >
                <span style={{ display: 'inline-flex' }}>
                  <Checkbox
                    label="Notify the difference"
                    checked={effectiveNotify}
                    disabled={noDrift}
                    onChange={(e) => setNotify(e.currentTarget.checked)}
                    data-testid="new-round-notify"
                  />
                </span>
              </Tooltip>
              {noDrift && (
                <Text size="xs" c="dimmed" data-testid="new-round-notify-explanation">
                  The checkout commit equals the previous approval commit, so there is no
                  difference to notify.
                </Text>
              )}

              <Checkbox
                label="Include diff"
                checked={includeDiff}
                disabled={!effectiveNotify}
                onChange={(e) => setIncludeDiff(e.currentTarget.checked)}
                data-testid="new-round-include-diff"
              />

              <CommentEditor
                label="Note"
                placeholder="Optional"
                value={note}
                onChange={setNote}
              />

              <Group justify="flex-end">
                <Button
                  variant="default"
                  disabled={!effectiveNotify || !startCommit}
                  onClick={() => openPreview('notification')}
                  data-testid="new-round-notification-preview-button"
                >
                  Preview notification
                </Button>
              </Group>
            </Stack>
          </Tabs.Panel>
        </Tabs>

        {error && (
          <Alert color="red" mt="sm" p="xs">
            <Text size="xs">{error}</Text>
          </Alert>
        )}

        {/* Matches the notify modal's footer: Preview and the action. There is no
            Cancel — the header's close button and Escape already leave. */}
        <Group justify="flex-end" pt="sm">
          <Button
            variant="default"
            disabled={!startCommit || !branch}
            onClick={() => openPreview('round')}
            data-testid="new-round-preview-button"
          >
            Preview
          </Button>
          <Button
            loading={posting}
            disabled={!canCreate}
            onClick={handleCreate}
            data-testid="new-round-submit"
          >
            Start Round {nextRoundIndex}
          </Button>
        </Group>

        <Modal
          opened={previewOpen}
          onClose={() => setPreviewOpen(false)}
          title="Round Preview"
          size={800}
          centered
          styles={{ header: { paddingTop: 12, paddingBottom: 12 }, body: { paddingBottom: 20 } }}
        >
          <Tabs
            value={activePreviewTab}
            onChange={(value) => setPreviewTab((value as PreviewTab | null) ?? 'round')}
            keepMounted={false}
          >
            <Tabs.List grow>
              <Tabs.Tab value="round">Round comment</Tabs.Tab>
              {/* Only rendered when a notification is going to be posted — a preview
                  of a comment nobody will send is a lie about what Start Round does. */}
              {effectiveNotify && <Tabs.Tab value="notification">Notification</Tabs.Tab>}
            </Tabs.List>

            <Tabs.Panel value="round" pt="sm">
              <div data-testid="new-round-preview">
                <PreviewFrame
                  isError={roundPreview.isError}
                  errorMessage={roundPreview.error?.message}
                  html={roundPreview.data}
                  frameTitle="Round Comment Preview"
                  errorTestId="new-round-preview-error"
                  loadingTestId="new-round-preview-loading"
                  errorLabel="Failed to render the round comment"
                />
              </div>
            </Tabs.Panel>

            <Tabs.Panel value="notification" pt="sm">
              <div data-testid="new-round-notification-preview">
                <PreviewFrame
                  isError={notificationPreview.isError}
                  errorMessage={notificationPreview.error?.message}
                  html={notificationPreview.data}
                  frameTitle="Round Notification Preview"
                  errorTestId="new-round-notification-preview-error"
                  loadingTestId="new-round-notification-preview-loading"
                  errorLabel="Failed to render the notification"
                />
              </div>
            </Tabs.Panel>
          </Tabs>
        </Modal>
        </>
      )}
    </Modal>
  )
}

/**
 * D23: facts read off the checkout, not inputs. A round always starts at the
 * checked-out commit on the checked-out branch — there is no commit picker in the
 * round flow, and a control that looks like a text box implies one exists.
 */
function CheckoutFacts({ branch, startCommit }: { branch: string; startCommit: string }) {
  return (
    <Stack
      gap={6}
      style={{
        border: '1px solid var(--mantine-color-gray-3)',
        borderRadius: 6,
        padding: '8px 12px',
        backgroundColor: 'var(--mantine-color-gray-0)',
      }}
    >
      <Text size="xs" c="dimmed">
        Read from the current checkout — a round starts at the checked-out commit.
      </Text>
      <Group gap="xl" align="flex-start" wrap="nowrap">
        <div>
          <Text size="xs" c="dimmed" fw={600}>Branch</Text>
          <Text size="sm" data-testid="new-round-branch">{branch || '—'}</Text>
        </div>
        <div style={{ minWidth: 0 }}>
          <Text size="xs" c="dimmed" fw={600}>Start commit</Text>
          <Text
            size="sm"
            data-testid="new-round-start-commit"
            style={{ fontFamily: 'monospace', wordBreak: 'break-all' }}
          >
            {startCommit || '—'}
          </Text>
        </div>
      </Group>
    </Stack>
  )
}

function PreviewFrame({
  isError,
  errorMessage,
  html,
  frameTitle,
  errorTestId,
  loadingTestId,
  errorLabel,
}: {
  isError: boolean
  errorMessage?: string
  html?: string
  frameTitle: string
  errorTestId: string
  loadingTestId: string
  errorLabel: string
}) {
  if (isError) {
    return (
      <Alert color="red" p="xs" data-testid={errorTestId}>
        <Text size="xs">{errorLabel}: {errorMessage}</Text>
      </Alert>
    )
  }
  if (html === undefined) {
    return (
      <Group justify="center" py="xl" data-testid={loadingTestId}>
        <Loader size="sm" />
      </Group>
    )
  }
  return (
    <iframe
      srcDoc={wrapInGithubStyles(html)}
      style={{ width: '100%', height: 460, border: '1px solid var(--mantine-color-gray-3)', borderRadius: 6 }}
      title={frameTitle}
    />
  )
}
