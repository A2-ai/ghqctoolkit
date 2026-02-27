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
import { IconChevronLeft, IconChevronRight } from '@tabler/icons-react'
import { useQueries, useQuery, useQueryClient } from '@tanstack/react-query'
import {
  type IssueStatusError,
  type IssueStatusResponse,
  type IssueStatusResult,
  fetchSingleIssueStatus,
  useAllMilestoneIssues,
} from '~/api/issues'
import { useMilestones } from '~/api/milestones'
import { type BranchCommit, fetchBranchCommits } from '~/api/commits'
import { CommitSlider } from './CommitSlider'

export type BareFileResolution =
  | { type: 'commit'; commit: string; file_name: string }
  | { type: 'issue'; issueNumber: number; commit: string; file_name: string }

const PAGE_SIZE = 10

interface BareFileResolveModalProps {
  opened: boolean
  onClose: () => void
  fileName: string
  referencingStatuses: IssueStatusResponse[]
  onResolve: (resolution: BareFileResolution) => void
}

export function BareFileResolveModal({
  opened,
  onClose,
  fileName,
  referencingStatuses,
  onResolve,
}: BareFileResolveModalProps) {
  const [currentPage, setCurrentPage] = useState(0)
  const [commitIdx, setCommitIdx] = useState(0)
  const [fileChangingOnly, setFileChangingOnly] = useState(false)
  const [initialized, setInitialized] = useState(false)

  const queryClient = useQueryClient()

  // ── Issue tab: load all milestones → all issues → filter by fileName ──────
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
      queryFn: async (): Promise<IssueStatusResult> => {
        try {
          const data = await fetchSingleIssueStatus(n)
          return { ok: true, data }
        } catch (e) {
          const err: IssueStatusError = {
            issue_number: n,
            kind: 'fetch_failed',
            error: (e as Error).message,
          }
          return { ok: false, error: err }
        }
      },
      staleTime: 5 * 60 * 1000,
      enabled: opened,
    })),
  })

  const matchingStatuses = matchingStatusQueries.flatMap(q => (q.data?.ok ? [q.data.data] : []))
  const isLoadingStatuses = matchingStatusQueries.some(
    q => q.isPending && q.fetchStatus !== 'idle',
  )

  // ── Commit tab ────────────────────────────────────────────────────────────

  const pinHash = useMemo(() => {
    if (referencingStatuses.length === 0) return undefined
    return (
      referencingStatuses[0].qc_status.approved_commit ??
      referencingStatuses[0].qc_status.latest_commit
    )
  }, [referencingStatuses])

  const distinctRefCommits = new Set(
    referencingStatuses.map(
      s => s.qc_status.approved_commit ?? s.qc_status.latest_commit,
    ),
  )
  const differentBranchesAmongReferencers =
    referencingStatuses.length > 0 &&
    !referencingStatuses.every(s => s.branch === referencingStatuses[0].branch)

  // Step 1: locate query — finds the page of the pin commit
  const { data: locateData } = useQuery({
    queryKey: ['branch-commits-locate', fileName, pinHash ?? '__none__'],
    queryFn: async () => {
      const result = await fetchBranchCommits({
        file: fileName,
        pageSize: PAGE_SIZE,
        ...(pinHash ? { locate: pinHash } : { page: 0 }),
      })
      queryClient.setQueryData(
        ['branch-commits', fileName, result.page, PAGE_SIZE],
        result,
      )
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

  // Step 2: regular page query
  const { data: pageData, isLoading: isPageLoading } = useQuery({
    queryKey: ['branch-commits', fileName, currentPage, PAGE_SIZE],
    queryFn: () =>
      fetchBranchCommits({ file: fileName, page: currentPage, pageSize: PAGE_SIZE }),
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
  const isLoading = !initialized || isPageLoading

  const visibleCommits = useMemo(() => {
    const indexed = rawCommits.map((c, i) => ({ ...c, origIdx: i })).reverse()
    if (!fileChangingOnly) return indexed
    return indexed.filter(c => c.file_changed || c.hash === pinHash)
  }, [rawCommits, fileChangingOnly, pinHash])

  useEffect(() => {
    if (visibleCommits.length === 0) return
    if (pinHash) {
      const pinIdx = visibleCommits.findIndex(c => c.hash === pinHash)
      if (pinIdx >= 0) { setCommitIdx(pinIdx); return }
    }
    setCommitIdx(visibleCommits.length - 1)
  }, [visibleCommits, pinHash]) // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => {
    if (!opened) {
      setCurrentPage(0)
      setCommitIdx(0)
      setFileChangingOnly(false)
      setInitialized(false)
    }
  }, [opened])

  function handleCommitConfirm() {
    const commit = visibleCommits[commitIdx]?.hash
    if (!commit) return
    onResolve({ type: 'commit', commit, file_name: fileName })
    onClose()
  }

  function handleIssueSelect(status: IssueStatusResponse) {
    const commit = status.qc_status.approved_commit ?? status.qc_status.latest_commit
    onResolve({ type: 'issue', issueNumber: status.issue.number, commit, file_name: fileName })
    onClose()
  }

  return (
    <Modal
      opened={opened}
      onClose={onClose}
      title={<Text fw={600} size="sm">Resolve bare file: {fileName}</Text>}
      size="lg"
    >
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
            {isLoading && (
              <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                <Loader size={14} />
                <Text size="sm" c="dimmed">Loading commits…</Text>
              </div>
            )}

            {!isLoading && (
              <>
                {distinctRefCommits.size > 1 && (
                  <Text size="xs" c="orange.7">
                    Multiple QC issues use different commits — defaulting to most recent
                  </Text>
                )}
                {differentBranchesAmongReferencers && (
                  <Text size="xs" c="orange.7">
                    Some referencing issues are on a different branch
                  </Text>
                )}

                <Checkbox
                  label="File changing commits only"
                  size="xs"
                  checked={fileChangingOnly}
                  onChange={e => setFileChangingOnly(e.currentTarget.checked)}
                />

                {visibleCommits.length === 0 ? (
                  <Text size="sm" c="dimmed">
                    {rawCommits.length === 0
                      ? 'No commits on this page'
                      : 'No file-changing commits on this page'}
                  </Text>
                ) : (
                  <>
                    <CommitSlider
                      commits={visibleCommits}
                      value={commitIdx}
                      onChange={setCommitIdx}
                      mb={40}
                    />
                    {visibleCommits[commitIdx] && (
                      <Text size="xs" c="dimmed" ff="monospace" style={{ wordBreak: 'break-all' }}>
                        {visibleCommits[commitIdx].message}
                      </Text>
                    )}
                    <Button
                      size="xs"
                      onClick={handleCommitConfirm}
                      disabled={!visibleCommits[commitIdx]}
                    >
                      Use commit {visibleCommits[commitIdx]?.hash.slice(0, 7)}
                    </Button>
                  </>
                )}

                {/* Pagination — left=Older (page+1), right=Newer (page-1) */}
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

                {pinHash && !isLoading && total > 0 && pageData && (
                  (() => {
                    const pinOnPage = rawCommits.some(c => c.hash === pinHash)
                    const locatePage = locateData?.page
                    return !pinOnPage && locatePage !== currentPage ? (
                      <Alert color="blue" p="xs">
                        <Text size="xs">
                          Default commit is on page {(locatePage ?? 0) + 1}.{' '}
                          <Anchor
                            size="xs"
                            onClick={() => setCurrentPage(locatePage ?? 0)}
                            style={{ cursor: 'pointer' }}
                          >
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
            {!isLoadingIssues && !isLoadingStatuses && matchingStatuses.length === 0 && (
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
                    <Anchor
                      href={s.issue.html_url}
                      target="_blank"
                      size="sm"
                      fw={700}
                      onClick={e => e.stopPropagation()}
                    >
                      {s.issue.title}
                    </Anchor>
                    {isApproved ? (
                      <Badge color="green" size="xs">Approved</Badge>
                    ) : (
                      <Badge color="yellow" size="xs">
                        {s.qc_status.status.replace(/_/g, ' ')}
                      </Badge>
                    )}
                  </div>
                  {s.issue.milestone && (
                    <Text size="xs" c="dimmed">Milestone: {s.issue.milestone}</Text>
                  )}
                  <Text size="xs" c="dimmed">Commit: {commit.slice(0, 7)}</Text>
                </div>
              )
            })}
          </Stack>
        </Tabs.Panel>
      </Tabs>
    </Modal>
  )
}
