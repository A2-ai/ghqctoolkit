import { AppShell, Menu, Text } from '@mantine/core'
import { useEffect, useRef, useState } from 'react'
import type { ReactNode } from 'react'
import { SwimLanes } from './SwimLanes'
import { CreateTab } from './CreateTab'
import { useRepoInfo } from '~/api/repo'
import { RepoStatus } from './RepoStatus'
import { MilestoneFilter } from './MilestoneFilter'
import { useMilestoneIssues } from '~/api/issues'
import {
  IconLayoutKanban,
  IconPlus,
  IconFileDescription,
  IconArchive,
  IconSettings,
  IconDots,
} from '@tabler/icons-react'

type Tab = 'status' | 'create' | 'record' | 'archive' | 'configuration'

const TABS: { id: Tab; label: string; icon: ReactNode }[] = [
  { id: 'status',        label: 'Status',        icon: <IconLayoutKanban size={15} /> },
  { id: 'create',        label: 'Create',        icon: <IconPlus size={15} /> },
  { id: 'record',        label: 'Record',        icon: <IconFileDescription size={15} /> },
  { id: 'archive',       label: 'Archive',       icon: <IconArchive size={15} /> },
  { id: 'configuration', label: 'Configuration', icon: <IconSettings size={15} /> },
]

const PRIMARY_TABS = TABS.slice(0, 2)
const MORE_TABS = TABS.slice(2)

function TabButton({
  tab,
  active,
  onClick,
  showIcon,
}: {
  tab: typeof TABS[number]
  active: boolean
  onClick: () => void
  showIcon: boolean
}) {
  return (
    <button
      onClick={onClick}
      style={{
        display: 'flex',
        alignItems: 'center',
        gap: 5,
        padding: '0 12px',
        height: '100%',
        background: 'none',
        border: 'none',
        cursor: 'pointer',
        borderBottom: active ? '2px solid #2f7a3b' : '2px solid transparent',
        fontWeight: active ? 600 : 400,
        fontSize: 14,
        color: active ? '#2f7a3b' : '#333',
        whiteSpace: 'nowrap',
      }}
    >
      {showIcon && tab.icon}
      {tab.label}
    </button>
  )
}

function MoreMenu({
  tabs,
  activeTab,
  setActiveTab,
}: {
  tabs: typeof TABS
  activeTab: Tab
  setActiveTab: (t: Tab) => void
}) {
  const anyActive = tabs.some((t) => t.id === activeTab)
  return (
    <Menu shadow="md">
      <Menu.Target>
        <button
          style={{
            display: 'flex',
            alignItems: 'center',
            gap: 5,
            padding: '0 12px',
            height: '100%',
            background: 'none',
            border: 'none',
            cursor: 'pointer',
            borderBottom: anyActive ? '2px solid #2f7a3b' : '2px solid transparent',
            fontWeight: anyActive ? 600 : 400,
            fontSize: 14,
            color: anyActive ? '#2f7a3b' : '#333',
            whiteSpace: 'nowrap',
          }}
        >
          <IconDots size={15} />
          More
        </button>
      </Menu.Target>
      <Menu.Dropdown>
        {tabs.map((tab) => (
          <Menu.Item
            key={tab.id}
            leftSection={tab.icon}
            onClick={() => setActiveTab(tab.id)}
          >
            {tab.label}
          </Menu.Item>
        ))}
      </Menu.Dropdown>
    </Menu>
  )
}

function PlaceholderTab({ tab }: { tab: string }) {
  return (
    <div
      style={{
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
        height: '60vh',
      }}
    >
      <Text c="dimmed" size="lg">
        {tab.charAt(0).toUpperCase() + tab.slice(1)} — coming soon
      </Text>
    </div>
  )
}

export function AppLayout() {
  const { data: repoData, isError: repoIsError, error: repoError } = useRepoInfo()
  const [selectedMilestones, setSelectedMilestones] = useState<number[]>([])
  const [includeClosedIssues, setIncludeClosedIssues] = useState(false)
  const [activeTab, setActiveTab] = useState<Tab>('status')
  const [headerWidth, setHeaderWidth] = useState(window.innerWidth)
  const [navWidth, setNavWidth] = useState(240)
  const headerInnerRef = useRef<HTMLDivElement>(null)
  const isDragging = useRef(false)
  const dragStartX = useRef(0)
  const dragStartWidth = useRef(0)

  useEffect(() => {
    const el = headerInnerRef.current
    if (!el) return
    const ro = new ResizeObserver(([entry]) => setHeaderWidth(entry.contentRect.width))
    ro.observe(el)
    return () => ro.disconnect()
  }, [])

  useEffect(() => {
    const onMouseMove = (e: MouseEvent) => {
      if (!isDragging.current) return
      const delta = e.clientX - dragStartX.current
      setNavWidth(Math.max(160, Math.min(520, dragStartWidth.current + delta)))
    }
    const onMouseUp = () => {
      if (!isDragging.current) return
      isDragging.current = false
      document.body.style.cursor = ''
      document.body.style.userSelect = ''
    }
    document.addEventListener('mousemove', onMouseMove)
    document.addEventListener('mouseup', onMouseUp)
    return () => {
      document.removeEventListener('mousemove', onMouseMove)
      document.removeEventListener('mouseup', onMouseUp)
    }
  }, [])

  const onDragHandleMouseDown = (e: React.MouseEvent) => {
    isDragging.current = true
    dragStartX.current = e.clientX
    dragStartWidth.current = navWidth
    document.body.style.cursor = 'col-resize'
    document.body.style.userSelect = 'none'
    e.preventDefault()
  }

  const showIcons = headerWidth > 820
  const showMore = headerWidth < 600

  const { statuses: rawStatuses, milestoneStatusByMilestone } = useMilestoneIssues(
    selectedMilestones,
    includeClosedIssues,
  )

  const dirtyFiles = new Set(repoData?.dirty_files ?? [])
  const statuses = rawStatuses.map((s) =>
    !s.dirty && dirtyFiles.has(s.issue.title) ? { ...s, dirty: true } : s
  )

  if (repoIsError) {
    const message = (repoError as Error)?.message ?? 'Failed to load repository information'
    return (
      <div
        style={{
          display: 'flex',
          flexDirection: 'column',
          alignItems: 'center',
          justifyContent: 'center',
          height: '100vh',
          gap: 24,
          backgroundColor: '#f8f9fa',
        }}
      >
        <img src="/logo.png" alt="ghqc logo" style={{ height: 80 }} />
        <div
          style={{
            backgroundColor: '#ffe3e3',
            border: '1px solid #ff8787',
            borderRadius: 8,
            padding: '20px 28px',
            maxWidth: 520,
            textAlign: 'center',
          }}
        >
          <Text fw={700} size="lg" c="#c92a2a" mb={8}>
            Unable to load repository
          </Text>
          <Text size="sm" c="#c92a2a">
            {message}
          </Text>
        </div>
      </div>
    )
  }

  return (
    <AppShell
      header={{ height: 88 }}
      navbar={{
        width: navWidth,
        breakpoint: 'sm',
        collapsed: { desktop: activeTab !== 'status' },
      }}
      padding="md"
    >
      <AppShell.Header style={{ backgroundColor: '#d7e7d3', borderBottom: 'none' }}>
        <div ref={headerInnerRef} style={{ display: 'flex', height: '100%' }}>
          {/* Left column: logo row + tab row */}
          <div
            style={{
              flex: 1,
              display: 'flex',
              flexDirection: 'column',
              overflow: 'hidden',
            }}
          >
            {/* Top row: logo + repo name */}
            <div
              style={{
                display: 'flex',
                alignItems: 'center',
                gap: 10,
                padding: '0 16px',
                height: 46,
              }}
            >
              <img src="/logo.png" alt="ghqc logo" style={{ height: 38 }} />
              {repoData && (
                <span style={{ fontSize: 20, fontWeight: 700 }}>
                  {repoData.owner} / {repoData.repo}
                </span>
              )}
            </div>

            {/* Tab bar */}
            <div
              style={{
                display: 'flex',
                alignItems: 'stretch',
                height: 42,
                paddingLeft: 8,
              }}
            >
              {PRIMARY_TABS.map((tab) => (
                <TabButton
                  key={tab.id}
                  tab={tab}
                  active={activeTab === tab.id}
                  onClick={() => setActiveTab(tab.id)}
                  showIcon={showIcons}
                />
              ))}

              {showMore ? (
                <MoreMenu
                  tabs={MORE_TABS}
                  activeTab={activeTab}
                  setActiveTab={setActiveTab}
                />
              ) : (
                MORE_TABS.map((tab) => (
                  <TabButton
                    key={tab.id}
                    tab={tab}
                    active={activeTab === tab.id}
                    onClick={() => setActiveTab(tab.id)}
                    showIcon={showIcons}
                  />
                ))
              )}
            </div>
          </div>

          {/* Right column: RepoStatus spanning full header height */}
          {repoData && (
            <div
              style={{
                display: 'flex',
                alignItems: 'center',
                padding: '0 16px',
                flexShrink: 0,
              }}
            >
              <RepoStatus data={repoData} />
            </div>
          )}
        </div>
      </AppShell.Header>

      <AppShell.Navbar style={{ padding: 0 }}>
        <div style={{ display: 'flex', height: '100%' }}>
          <div style={{ flex: 1, padding: 'var(--mantine-spacing-md)', overflowY: 'auto' }}>
            <MilestoneFilter
              selected={selectedMilestones}
              onSelect={setSelectedMilestones}
              includeClosedIssues={includeClosedIssues}
              onIncludeClosedIssuesChange={setIncludeClosedIssues}
              milestoneStatusByMilestone={milestoneStatusByMilestone}
            />
          </div>
          <div
            onMouseDown={onDragHandleMouseDown}
            style={{ width: 6, flexShrink: 0, cursor: 'col-resize' }}
          />
        </div>
      </AppShell.Navbar>

      <AppShell.Main>
        {activeTab === 'status' && (
          <SwimLanes
            statuses={statuses}
            currentBranch={repoData?.branch ?? ''}
            remoteCommit={repoData?.remote_commit ?? ''}
          />
        )}
        {activeTab === 'create' && (
          <div style={{
            margin: 'calc(-1 * var(--mantine-spacing-md))',
            height: 'calc(100vh - 88px)',
          }}>
            <CreateTab />
          </div>
        )}
        {activeTab !== 'status' && activeTab !== 'create' && (
          <PlaceholderTab tab={activeTab} />
        )}
      </AppShell.Main>
    </AppShell>
  )
}
