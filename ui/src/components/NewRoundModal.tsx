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
import type { CreateRoundResponse, IssueStatusResponse, RoundInfo } from '~/api/issues'
import { latestRound, postRound, roundApprovedCommit } from '~/api/issues'
import type { RoundPreviewRequest } from '~/api/preview'
import { useRoundPreview } from '~/api/preview'
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

  // D47: the preview is rendered server-side by the same `QCRound::generate_body`
  // the creation posts, so it cannot drift from what lands on the issue — and only
  // the server can emit the `[file contents at initial qc commit]` link. The round
  // index is not sent: the server derives it exactly as `POST /rounds` does.
  //
  // Gated on the Preview tab being open and on debounced checklist fields — the
  // checklist is an editable textarea and this must not be a request per keystroke.
  const [debouncedChecklistName] = useDebouncedValue(checklistName, 400)
  const [debouncedChecklistContent] = useDebouncedValue(checklistContent, 400)
  const previewRequest = useMemo<RoundPreviewRequest | null>(() => {
    if (activeTab !== 'preview' || !startCommit || !branch) return null
    return {
      issue_number: issue.number,
      start_commit: startCommit,
      branch,
      checklist: { name: debouncedChecklistName.trim(), content: debouncedChecklistContent },
    }
  }, [activeTab, startCommit, branch, issue.number, debouncedChecklistName, debouncedChecklistContent])
  const preview = useRoundPreview(previewRequest)

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
        <Tabs value={activeTab} onChange={setActiveTab} keepMounted={false}>
          <Tabs.List grow>
            <Tabs.Tab value="round">Round</Tabs.Tab>
            <Tabs.Tab value="checklist">Checklist</Tabs.Tab>
            <Tabs.Tab value="preview">Preview</Tabs.Tab>
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

              {/* D23: both come from the checkout, read-only. No commit picker. */}
              <TextInput label="Branch" value={branch} readOnly data-testid="new-round-branch" />
              <TextInput
                label="Start commit"
                description="Taken from the current checkout"
                value={startCommit}
                readOnly
                data-testid="new-round-start-commit"
              />

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
            </Stack>
          </Tabs.Panel>

          <Tabs.Panel value="checklist" pt="md">
            <Stack gap="sm">
              {/* Shown only when there is more than one prior round to seed from. */}
              {rounds.length > 1 && (
                <Select
                  label="Base round"
                  description="Seeds the checklist below"
                  data={rounds.map((r) => ({
                    value: String(r.index),
                    label: `Round ${r.index}${r.checklist_name ? ` — ${r.checklist_name}` : ''}`,
                  }))}
                  value={String(baseRound.index)}
                  onChange={handleBaseRoundChange}
                  allowDeselect={false}
                  data-testid="new-round-base-round"
                />
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

          <Tabs.Panel value="preview" pt="md">
            <div data-testid="new-round-preview">
              {preview.isError ? (
                <Alert color="red" p="xs" data-testid="new-round-preview-error">
                  <Text size="xs">Failed to render the round comment: {preview.error.message}</Text>
                </Alert>
              ) : preview.data === undefined ? (
                <Group justify="center" py="xl" data-testid="new-round-preview-loading">
                  <Loader size="sm" />
                </Group>
              ) : (
                <iframe
                  srcDoc={wrapInGithubStyles(preview.data)}
                  style={{ width: '100%', height: 460, border: '1px solid var(--mantine-color-gray-3)', borderRadius: 6 }}
                  title="Round Comment Preview"
                />
              )}
            </div>
          </Tabs.Panel>

          {error && (
            <Alert color="red" mt="sm" p="xs">
              <Text size="xs">{error}</Text>
            </Alert>
          )}

          <Group justify="flex-end" pt="sm">
            <Button variant="default" onClick={onClose}>Cancel</Button>
            <Button
              loading={posting}
              disabled={!canCreate}
              onClick={handleCreate}
              data-testid="new-round-submit"
            >
              Start Round {nextRoundIndex}
            </Button>
          </Group>
        </Tabs>
      )}
    </Modal>
  )
}
