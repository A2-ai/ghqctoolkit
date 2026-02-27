/**
 * Unified modal for resolving a file to a specific commit.
 *
 * Two modes:
 *   • Bare-file mode  – `fileName` is pre-supplied; opens directly on the
 *                       commit/issue step. Accepts optional `referencingStatuses`
 *                       to seed the default commit.
 *   • Add-file mode   – `fileName` is undefined; shows a file-picker step first,
 *                       then advances to the commit/issue step.
 */

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
  Stack,
  Tabs,
  Text,
} from '@mantine/core'
import { IconArrowLeft, IconChevronLeft, IconChevronRight } from '@tabler/icons-react'
import { useQueries, useQuery, useQueryClient } from '@tanstack/react-query'
import {
  type IssueStatusError,
  type IssueStatusResponse,
  type IssueStatusResult,
  issueStatusBatcher,
  useAllMilestoneIssues,
} from '~/api/issues'
import { useMilestones } from '~/api/milestones'
import { type BranchCommit, fetchBranchCommits } from '~/api/commits'
import { CommitSlider } from './CommitSlider'
import { FileTreeBrowser } from './FileTreeBrowser'

export interface FileResolution {
  file_name: string
  commit: string
  /** Set when the user resolved via the Issue tab — enables same-issue de-dup. */
  source_issue_number?: number
}

const PAGE_SIZE = 10

interface FileResolveModalProps {
  opened: boolean
  onClose: () => void
  /** Pre-supplied in bare-file mode. Omit to show file-picker step first. */
  fileName?: string
  /** Bare-file mode only: QC issues that reference this file (seeds default commit). */
  referencingStatuses?: IssueStatusResponse[]
  /** Add-file mode only: files already rendered — greyed out in the picker. */
  claimedFiles?: Set<string>
  /** Controls the title when fileName is pre-supplied. Defaults to 'resolve'. */
  editMode?: 'resolve' | 'edit'
  onResolve: (resolution: FileResolution) => void
}

export function FileResolveModal({
  opened,
  onClose,
  fileName: fileNameProp,
  referencingStatuses = [],
  claimedFiles = new Set(),
  editMode = 'resolve',
  onResolve,
}: FileResolveModalProps) {
  const addFileMode = fileNameProp === undefined

  const [step, setStep] = useState<1 | 2>(addFileMode ? 1 : 2)
  const [pickedFile, setPickedFile] = useState<string | null>(null)

  const fileName = addFileMode ? pickedFile : fileNameProp

  // Sync step with mode on every open/close transition
  useEffect(() => {
    if (opened) {
      setStep(addFileMode ? 1 : 2)
    } else {
      setPickedFile(null)
    }
  }, [opened, addFileMode])

  const title =
    step === 1
      ? 'Add file to archive'
      : addFileMode
        ? `Add file: ${fileName}`
        : editMode === 'edit'
          ? `Edit: ${fileName}`
          : `Resolve: ${fileName}`

  return (
    <Modal
      opened={opened}
      onClose={onClose}
      title={<Text fw={600} size="sm">{title}</Text>}
      size="lg"
    >
      {step === 1 && (
        <Stack gap="sm">
          <Text size="xs" c="dimmed">
            Select a file to add. Files already in the archive are greyed out.
          </Text>
          <div style={{ border: '1px solid var(--mantine-color-gray-3)', borderRadius: 6, padding: 8 }}>
            <FileTreeBrowser
              selectedFile={pickedFile}
              onSelect={setPickedFile}
              claimedFiles={claimedFiles}
            />
          </div>
          <Button size="xs" disabled={!pickedFile} onClick={() => setStep(2)}>
            Next →
          </Button>
        </Stack>
      )}

      {step === 2 && fileName && (
        <CommitIssueStep
          key={fileName}
          fileName={fileName}
          opened={opened}
          referencingStatuses={referencingStatuses}
          showBack={addFileMode}
          onBack={() => setStep(1)}
          onResolve={(commit, issueNumber) => {
            onResolve({ file_name: fileName, commit, source_issue_number: issueNumber })
            onClose()
          }}
        />
      )}
    </Modal>
  )
}

// ─── CommitIssueStep ──────────────────────────────────────────────────────────

interface CommitIssueStepProps {
  fileName: string
  opened: boolean
  referencingStatuses: IssueStatusResponse[]
  showBack: boolean
  onBack: () => void
  onResolve: (commit: string, issueNumber?: number) => void
}

function CommitIssueStep({
  fileName,
  opened,
  referencingStatuses,
  showBack,
  onBack,
  onResolve,
}: CommitIssueStepProps) {
  const [currentPage, setCurrentPage] = useState(0)
  const [commitIdx, setCommitIdx] = useState(0)
  const [showAll, setShowAll] = useState(false)
  const [initialized, setInitialized] = useState(false)

  const queryClient = useQueryClient()

  // Pin hash: most recent commit from referencing statuses (if any)
  const pinHash = useMemo(() => {
    if (referencingStatuses.length === 0) return undefined
    return (
      referencingStatuses[0].qc_status.approved_commit ??
      referencingStatuses[0].qc_status.latest_commit
    )
  }, [referencingStatuses])

  const distinctRefCommits = new Set(
    referencingStatuses.map(s => s.qc_status.approved_commit ?? s.qc_status.latest_commit),
  )
  const differentBranchesAmongReferencers =
    referencingStatuses.length > 0 &&
    !referencingStatuses.every(s => s.branch === referencingStatuses[0].branch)

  // ── Commit tab ─────────────────────────────────────────────────────────────

  const { data: locateData } = useQuery({
    queryKey: ['branch-commits-locate', fileName, pinHash ?? '__none__'],
    queryFn: async () => {
      const result = await fetchBranchCommits({
        file: fileName,
        pageSize: PAGE_SIZE,
        ...(pinHash ? { locate: pinHash } : { page: 0 }),
      })
      queryClient.setQueryData(['branch-commits', fileName, result.page, PAGE_SIZE], result)
      return result
    },
    enabled: opened,
    staleTime: 5 * 60 * 1000,
  })

  useEffect(() => {
    if (locateData && !initialized) {
      setCurrentPage(typeof locateData.page === 'number' ? locateData.page : 0)
      setInitialized(true)
    }
  }, [locateData, initialized])

  const { data: pageData, isLoading: isPageLoading } = useQuery({
    queryKey: ['branch-commits', fileName, currentPage, PAGE_SIZE],
    queryFn: () => fetchBranchCommits({ file: fileName, page: currentPage, pageSize: PAGE_SIZE }),
    enabled: opened && initialized,
    staleTime: 5 * 60 * 1000,
  })

  const rawCommits: BranchCommit[] = Array.isArray(pageData?.commits)
    ? pageData!.commits
    : Array.isArray(locateData?.commits)
      ? locateData!.commits
      : []
  const total =
    typeof pageData?.total === 'number'
      ? pageData.total
      : typeof locateData?.total === 'number'
        ? locateData.total
        : 0
  const totalPages = Math.ceil(total / PAGE_SIZE)
  const isLoadingCommits = !initialized || isPageLoading

  const visibleCommits = useMemo(() => {
    const indexed = rawCommits.map((c, i) => ({ ...c, origIdx: i })).reverse()
    if (showAll) return indexed
    return indexed.filter(c => c.file_changed || c.hash === pinHash)
  }, [rawCommits, showAll, pinHash])

  useEffect(() => {
    if (visibleCommits.length === 0) return
    if (pinHash) {
      const pinIdx = visibleCommits.findIndex(c => c.hash === pinHash)
      if (pinIdx >= 0) { setCommitIdx(pinIdx); return }
    }
    setCommitIdx(visibleCommits.length - 1)
  }, [visibleCommits, pinHash]) // eslint-disable-line react-hooks/exhaustive-deps

  // ── Issue tab ──────────────────────────────────────────────────────────────

  const { data: allMilestones } = useMilestones()
  const allMilestoneNumbers = useMemo(
    () => (allMilestones ?? []).map(m => m.number),
    [allMilestones],
  )

  const { issues: allIssues, isLoading: isLoadingIssues } = useAllMilestoneIssues(
    allMilestoneNumbers,
    opened,
  )

  const matchingIssueNumbers = useMemo(
    () => allIssues.filter(i => i.title === fileName).map(i => i.number),
    [allIssues, fileName],
  )

  const matchingStatusQueries = useQueries({
    queries: matchingIssueNumbers.map(n => ({
      queryKey: ['issue', 'status', n],
      queryFn: () => issueStatusBatcher.load(n),
      staleTime: 5 * 60 * 1000,
      enabled: opened && !isLoadingIssues,
    })),
  })

  const isLoadingStatuses = isLoadingIssues || matchingStatusQueries.some(q => q.isPending && q.fetchStatus !== 'idle')

  const { approvedStatuses, otherStatuses, statusErrors } = useMemo(() => {
    const approved: IssueStatusResponse[] = []
    const other: IssueStatusResponse[] = []
    const errors: IssueStatusError[] = []

    for (const q of matchingStatusQueries) {
      if (!q.data) continue
      if (q.data.ok) {
        const s = q.data.data
        const isApproved =
          s.qc_status.status === 'approved' ||
          s.qc_status.status === 'changes_after_approval'
        if (isApproved) approved.push(s)
        else other.push(s)
      } else {
        errors.push(q.data.error)
      }
    }

    // Sort each group by issue number descending (highest first)
    approved.sort((a, b) => b.issue.number - a.issue.number)
    other.sort((a, b) => b.issue.number - a.issue.number)

    return { approvedStatuses: approved, otherStatuses: other, statusErrors: errors }
  }, [matchingStatusQueries])

  const matchingStatuses = useMemo(
    () => [...approvedStatuses, ...otherStatuses],
    [approvedStatuses, otherStatuses],
  )

  function handleIssueSelect(status: IssueStatusResponse) {
    onResolve(
      status.qc_status.approved_commit ?? status.qc_status.latest_commit,
      status.issue.number,
    )
  }

  return (
    <Stack gap="sm">
      {showBack && (
        <Button
          size="xs"
          variant="subtle"
          leftSection={<IconArrowLeft size={12} />}
          onClick={onBack}
          style={{ alignSelf: 'flex-start' }}
        >
          Back
        </Button>
      )}

      <Tabs defaultValue="commit">
        <Tabs.List>
          <Tabs.Tab value="commit">Select Commit</Tabs.Tab>
          <Tabs.Tab value="issue">
            Select Issue
            {isLoadingIssues || isLoadingStatuses
              ? <Loader size={10} ml={6} />
              : matchingStatuses.length > 0
                ? <Text span size="xs" c="dimmed" ml={4}>({matchingStatuses.length})</Text>
                : null}
          </Tabs.Tab>
        </Tabs.List>

        <Tabs.Panel value="commit" pt="md">
          <Stack gap="sm">
            {isLoadingCommits ? (
              <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                <Loader size={14} />
                <Text size="sm" c="dimmed">Loading commits…</Text>
              </div>
            ) : (
              <>
                {distinctRefCommits.size > 1 && (
                  <Text size="xs" c="orange.7">
                    Multiple QC issues use different commits — defaulting to most recent
                  </Text>
                )}
                {differentBranchesAmongReferencers && (
                  <Text size="xs" c="orange.7">Some referencing issues are on a different branch</Text>
                )}

                <Checkbox
                  label="Show all commits"
                  size="xs"
                  checked={showAll}
                  onChange={e => setShowAll(e.currentTarget.checked)}
                />

                {visibleCommits.length === 0 ? (
                  <Text size="sm" c="dimmed">
                    {rawCommits.length === 0 ? 'No commits on this page' : 'No file-changing commits on this page'}
                  </Text>
                ) : (
                  <>
                    <CommitSlider commits={visibleCommits} value={commitIdx} onChange={setCommitIdx} mb={40} />
                    {visibleCommits[commitIdx] && (
                      <Text size="xs" c="dimmed" ff="monospace" style={{ wordBreak: 'break-all' }}>
                        {visibleCommits[commitIdx].message}
                      </Text>
                    )}
                    <Button
                      size="xs"
                      onClick={() => onResolve(visibleCommits[commitIdx]?.hash ?? '')}
                      disabled={!visibleCommits[commitIdx]}
                    >
                      Use commit {visibleCommits[commitIdx]?.hash.slice(0, 7)}
                    </Button>
                  </>
                )}

                <Group justify="space-between" mt={4}>
                  <Button
                    size="xs"
                    variant="subtle"
                    leftSection={<IconChevronLeft size={12} />}
                    disabled={currentPage >= totalPages - 1}
                    onClick={() => setCurrentPage(p => p + 1)}
                  >
                    Older
                  </Button>
                  <Text size="xs" c="dimmed">
                    {currentPage * PAGE_SIZE + 1}–{Math.min((currentPage + 1) * PAGE_SIZE, total)} of {total}
                  </Text>
                  <Button
                    size="xs"
                    variant="subtle"
                    rightSection={<IconChevronRight size={12} />}
                    disabled={currentPage === 0}
                    onClick={() => setCurrentPage(p => p - 1)}
                  >
                    Newer
                  </Button>
                </Group>

                {pinHash && !isLoadingCommits && total > 0 && pageData && (
                  (() => {
                    const pinOnPage = rawCommits.some(c => c.hash === pinHash)
                    const locatePage = locateData?.page
                    return !pinOnPage && locatePage !== currentPage ? (
                      <Alert color="blue" p="xs">
                        <Text size="xs">
                          Default commit is on page {(locatePage ?? 0) + 1}.{' '}
                          <Anchor size="xs" onClick={() => setCurrentPage(locatePage ?? 0)} style={{ cursor: 'pointer' }}>
                            Go there
                          </Anchor>
                        </Text>
                      </Alert>
                    ) : null
                  })()
                )}
              </>
            )}
          </Stack>
        </Tabs.Panel>

        <Tabs.Panel value="issue" pt="md">
          <Stack gap="sm">
            {(isLoadingIssues || isLoadingStatuses) && (
              <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                <Loader size={14} />
                <Text size="sm" c="dimmed">
                  {isLoadingIssues ? 'Loading issues across all milestones…' : 'Loading statuses…'}
                </Text>
              </div>
            )}
            {!isLoadingIssues && !isLoadingStatuses && matchingStatuses.length === 0 && statusErrors.length === 0 && (
              <Text size="sm" c="dimmed">No QC issues found for this file across all milestones</Text>
            )}
            {matchingStatuses.map(s => {
              const isApproved =
                s.qc_status.status === 'approved' ||
                s.qc_status.status === 'changes_after_approval'
              const commit = s.qc_status.approved_commit ?? s.qc_status.latest_commit
              return (
                <div
                  key={s.issue.number}
                  onClick={() => handleIssueSelect(s)}
                  style={{
                    cursor: 'pointer',
                    padding: '8px 12px',
                    borderRadius: 6,
                    border: '1px solid var(--mantine-color-gray-3)',
                    backgroundColor: 'white',
                  }}
                >
                  <div style={{ display: 'flex', alignItems: 'center', gap: 8, flexWrap: 'wrap' }}>
                    <Anchor href={s.issue.html_url} target="_blank" size="sm" fw={700} onClick={e => e.stopPropagation()}>
                      {s.issue.title}
                    </Anchor>
                    {isApproved
                      ? <Badge color="green" size="xs">Approved</Badge>
                      : <Badge color="yellow" size="xs">{s.qc_status.status.replace(/_/g, ' ')}</Badge>}
                  </div>
                  {s.issue.milestone && <Text size="xs" c="dimmed">Milestone: {s.issue.milestone}</Text>}
                  <Text size="xs" c="dimmed">Commit: {commit.slice(0, 7)}</Text>
                </div>
              )
            })}
            {statusErrors.map(err => (
              <Alert key={err.issue_number} color="red" p="xs">
                <Text size="xs">Issue #{err.issue_number}: {err.error}</Text>
              </Alert>
            ))}
          </Stack>
        </Tabs.Panel>
      </Tabs>
    </Stack>
  )
}
