import { ActionIcon, AppShell, Menu, Text, Tooltip } from '@mantine/core'
import { useEffect, useLayoutEffect, useRef, useState } from 'react'
import type { ReactNode } from 'react'
import { Outlet, useLocation, useNavigate } from '@tanstack/react-router'
import { useRepoInfo } from '~/api/repo'
import { useConfigurationStatus } from '~/api/configuration'
import { RepoStatus } from './RepoStatus'
import { MilestoneFilter } from './MilestoneFilter'
import { useMilestoneIssues } from '~/api/issues'
import {
  closeArchiveModals,
  closeCreateModals,
  closeRecordModals,
  useUiSession,
} from '~/state/uiSession'
import {
  IconLayoutKanban,
  IconPlus,
  IconFileDescription,
  IconArchive,
  IconSettings,
  IconDots,
  IconInfoCircle,
  IconChevronLeft,
  IconChevronRight,
} from '@tabler/icons-react'

type Tab = 'status' | 'create' | 'record' | 'archive' | 'configuration'

const TABS: { id: Tab; label: string; icon: ReactNode; to: string }[] = [
  { id: 'status',        label: 'Status',        icon: <IconLayoutKanban size={15} />, to: '/status' },
  { id: 'create',        label: 'Create',        icon: <IconPlus size={15} />, to: '/create' },
  { id: 'record',        label: 'Record',        icon: <IconFileDescription size={15} />, to: '/record' },
  { id: 'archive',       label: 'Archive',       icon: <IconArchive size={15} />, to: '/archive' },
  { id: 'configuration', label: 'Configuration', icon: <IconSettings size={15} />, to: '/configuration' },
]

const PRIMARY_TABS = TABS.slice(0, 2)
const MORE_TABS = TABS.slice(2)

function TabButton({
  tab,
  active,
  onClick,
  showIcon,
  warning,
}: {
  tab: typeof TABS[number]
  active: boolean
  onClick: () => void
  showIcon: boolean
  warning?: string
}) {
  const color = active ? '#2f7a3b' : '#333'
  return (
    <Tooltip label={warning ?? ''} disabled={!warning}>
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
          borderBottom: active ? `2px solid ${color}` : '2px solid transparent',
          fontWeight: active ? 600 : 400,
          fontSize: 14,
          color,
          whiteSpace: 'nowrap',
        }}
      >
        {warning ? (
          <span style={{
            display: 'inline-flex',
            alignItems: 'center',
            gap: 3,
            backgroundColor: '#fff3bf',
            border: '1px solid #f59f00',
            borderRadius: 10,
            padding: '1px 7px',
          }}>
            {showIcon && tab.icon}
            {tab.label}
            <IconInfoCircle size={12} color="#e67700" style={{ flexShrink: 0 }} />
          </span>
        ) : (
          <>
            {showIcon && tab.icon}
            {tab.label}
          </>
        )}
      </button>
    </Tooltip>
  )
}

function MoreMenu({
  tabs,
  activeTab,
  setActiveTab,
  warnings,
}: {
  tabs: typeof TABS
  activeTab: Tab
  setActiveTab: (t: Tab) => void
  warnings: Partial<Record<Tab, string>>
}) {
  const anyActive = tabs.some((t) => t.id === activeTab)
  const menuColor = anyActive ? '#2f7a3b' : '#333'
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
            borderBottom: anyActive ? `2px solid ${menuColor}` : '2px solid transparent',
            fontWeight: anyActive ? 600 : 400,
            fontSize: 14,
            color: menuColor,
            whiteSpace: 'nowrap',
          }}
        >
          <IconDots size={15} />
          More
        </button>
      </Menu.Target>
      <Menu.Dropdown>
        {tabs.map((tab) => {
          const warn = warnings[tab.id]
          return (
            <Tooltip key={tab.id} label={warn ?? ''} disabled={!warn} position="right">
              <Menu.Item leftSection={tab.icon} onClick={() => setActiveTab(tab.id)}>
                {warn ? (
                  <span style={{
                    display: 'inline-flex',
                    alignItems: 'center',
                    gap: 3,
                    backgroundColor: '#fff3bf',
                    border: '1px solid #f59f00',
                    borderRadius: 10,
                    padding: '1px 7px',
                  }}>
                    {tab.label}
                    <IconInfoCircle size={12} color="#e67700" style={{ flexShrink: 0 }} />
                  </span>
                ) : tab.label}
              </Menu.Item>
            </Tooltip>
          )
        })}
      </Menu.Dropdown>
    </Menu>
  )
}

export function AppLayout() {
  const { data: repoData, isError: repoIsError, error: repoError } = useRepoInfo()
  const { data: configStatus } = useConfigurationStatus()
  const navigate = useNavigate()
  const location = useLocation()
  const { status, setStatus, setCreate, setRecord, setArchive } = useUiSession()
  const [headerWidth, setHeaderWidth] = useState(0)
  const lastNavWidthRef = useRef(status.navWidth)
  const headerInnerRef = useRef<HTMLDivElement>(null)
  const lastPathRef = useRef(location.pathname)
  const isDragging = useRef(false)
  const dragStartX = useRef(0)
  const dragStartWidth = useRef(0)

  const activeTab = (() => {
    const path = location.pathname
    if (path.startsWith('/create')) return 'create'
    if (path.startsWith('/record')) return 'record'
    if (path.startsWith('/archive')) return 'archive'
    if (path.startsWith('/configuration')) return 'configuration'
    return 'status'
  })()

  const closeTransientUi = () => {
    setCreate(closeCreateModals)
    setRecord(closeRecordModals)
    setArchive(closeArchiveModals)
  }

  const navigateToTab = (to: string) => {
    if (to === location.pathname) return
    closeTransientUi()
    navigate({ to })
  }

  useEffect(() => {
    setHeaderWidth(window.innerWidth)
    const el = headerInnerRef.current
    if (!el) return
    const ro = new ResizeObserver(([entry]) => setHeaderWidth(entry.contentRect.width))
    ro.observe(el)
    return () => ro.disconnect()
  }, [])

  useLayoutEffect(() => {
    if (lastPathRef.current === location.pathname) return
    lastPathRef.current = location.pathname
    closeTransientUi()
  }, [location.pathname, setCreate, setRecord, setArchive])

  useEffect(() => {
    const onMouseMove = (e: MouseEvent) => {
      if (!isDragging.current) return
      const delta = e.clientX - dragStartX.current
      setStatus((prev) => ({
        ...prev,
        navWidth: Math.max(160, Math.min(520, dragStartWidth.current + delta)),
      }))
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
    dragStartWidth.current = status.navWidth
    document.body.style.cursor = 'col-resize'
    document.body.style.userSelect = 'none'
    e.preventDefault()
  }

  const showIcons = headerWidth > 820
  const showMore = headerWidth < 600

  const tabWarnings: Partial<Record<Tab, string>> = {}
  if (configStatus && !configStatus.exists && configStatus.git_repository === null) {
    tabWarnings.configuration = 'Configuration repository is not set up'
  }

  const { milestoneStatusByMilestone } = useMilestoneIssues(
    status.selectedMilestones,
    status.includeClosedIssues,
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
        <img src="./logo.png" alt="ghqc logo" style={{ height: 80 }} />
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
        width: status.navCollapsed ? 28 : status.navWidth,
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
              <img src="./logo.png" alt="ghqc logo" style={{ height: 38 }} />
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
                  onClick={() => navigateToTab(tab.to)}
                  showIcon={showIcons}
                  warning={tabWarnings[tab.id]}
                />
              ))}

              {showMore ? (
                <MoreMenu
                  tabs={MORE_TABS}
                  activeTab={activeTab}
                  setActiveTab={(tab) => {
                    const next = TABS.find((entry) => entry.id === tab)
                    if (next) navigateToTab(next.to)
                  }}
                  warnings={tabWarnings}
                />
              ) : (
                MORE_TABS.map((tab) => (
                  <TabButton
                    key={tab.id}
                    tab={tab}
                    active={activeTab === tab.id}
                    onClick={() => navigateToTab(tab.to)}
                    showIcon={showIcons}
                    warning={tabWarnings[tab.id]}
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
          {!status.navCollapsed && (
            <div style={{ flex: 1, overflow: 'hidden', minHeight: 0 }}>
              <MilestoneFilter
                selected={status.selectedMilestones}
                onSelect={(selectedMilestones) => setStatus((prev) => ({ ...prev, selectedMilestones }))}
                includeClosedIssues={status.includeClosedIssues}
                onIncludeClosedIssuesChange={(includeClosedIssues) => setStatus((prev) => ({ ...prev, includeClosedIssues }))}
                milestoneStatusByMilestone={milestoneStatusByMilestone}
              />
            </div>
          )}
          <div style={{ width: 28, flexShrink: 0, display: 'flex', flexDirection: 'column', alignItems: 'center' }}>
            <ActionIcon
              variant="subtle"
              size="sm"
              onClick={() => {
                if (status.navCollapsed) {
                  setStatus((prev) => ({ ...prev, navWidth: lastNavWidthRef.current }))
                } else {
                  lastNavWidthRef.current = status.navWidth
                }
                setStatus((prev) => ({ ...prev, navCollapsed: !prev.navCollapsed }))
              }}
              style={{ marginTop: 8 }}
              title={status.navCollapsed ? 'Expand' : 'Collapse'}
            >
              {status.navCollapsed ? <IconChevronRight size={14} /> : <IconChevronLeft size={14} />}
            </ActionIcon>
            {!status.navCollapsed && (
              <div
                onMouseDown={onDragHandleMouseDown}
                style={{ flex: 1, width: '100%', cursor: 'col-resize' }}
              />
            )}
          </div>
        </div>
      </AppShell.Navbar>

      <AppShell.Main
        style={{
          display: 'flex',
          flexDirection: 'column',
          minHeight: '100dvh',
          height: '100dvh',
          boxSizing: 'border-box',
        }}
      >
        <div style={{ flex: 1, minHeight: 0, overflow: 'hidden' }}>
          <Outlet />
        </div>
      </AppShell.Main>
    </AppShell>
  )
}
