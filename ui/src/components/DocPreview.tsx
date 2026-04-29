import { useEffect, useRef, useState } from 'react'
import { ActionIcon, Group, Loader, Tabs, Text, Tooltip } from '@mantine/core'
import { IconArrowsMaximize, IconMinus, IconPlus, IconRefresh } from '@tabler/icons-react'
import { renderAsync } from 'docx-preview'
import * as XLSX from 'xlsx'

type DocFormat = 'docx' | 'xlsx' | 'pdf' | 'image' | 'unsupported'

function detectFormat(fileName: string): DocFormat {
  const ext = fileName.split('.').pop()?.toLowerCase()
  if (ext === 'docx' || ext === 'doc') return 'docx'
  if (ext === 'xlsx' || ext === 'xls' || ext === 'csv') return 'xlsx'
  if (ext === 'pdf') return 'pdf'
  if (ext && ['png', 'jpg', 'jpeg', 'gif', 'bmp', 'webp'].includes(ext)) return 'image'
  return 'unsupported'
}

interface Props {
  url: string
  fileName: string
  height?: number | string
}

interface XlsxSheet {
  name: string
  html: string
}

const ZOOM_STEP = 0.1
const MIN_ZOOM = 0.25
const MAX_ZOOM = 3

export function DocPreview({ url, fileName, height = 500 }: Props) {
  const format = detectFormat(fileName)
  const docxRef = useRef<HTMLDivElement | null>(null)
  const wrapperRef = useRef<HTMLElement | null>(null)
  const [loading, setLoading] = useState(format === 'docx' || format === 'xlsx')
  const [error, setError] = useState<string | null>(null)
  const [sheets, setSheets] = useState<XlsxSheet[]>([])
  const [activeSheet, setActiveSheet] = useState<string | null>(null)
  // null = "fit to width" (auto). number = explicit user zoom.
  const [userZoom, setUserZoom] = useState<number | null>(null)
  const [fitScale, setFitScale] = useState<number>(1)

  // Render docx and set up a ResizeObserver that recomputes the fit-to-width scale.
  useEffect(() => {
    if (format !== 'docx') return
    let cancelled = false
    let resizeObserver: ResizeObserver | null = null
    setLoading(true)
    setError(null)
    setUserZoom(null)
    fetch(url)
      .then((res) => {
        if (!res.ok) throw new Error(`Failed to fetch (${res.status})`)
        return res.blob()
      })
      .then(async (blob) => {
        if (cancelled || !docxRef.current) return
        docxRef.current.innerHTML = ''
        await renderAsync(blob, docxRef.current, undefined, {
          inWrapper: true,
          ignoreWidth: false,
          ignoreHeight: false,
          breakPages: true,
          renderHeaders: false,
          renderFooters: false,
          experimental: true,
        })
        if (cancelled || !docxRef.current) return
        const host = docxRef.current
        const wrapper = host.querySelector<HTMLElement>('.docx-wrapper')
        const sections = Array.from(host.querySelectorAll<HTMLElement>('section.docx'))
        if (!wrapper || sections.length === 0) return
        wrapperRef.current = wrapper
        const observed = host.parentElement ?? host
        const widestPage = sections.reduce((max, s) => Math.max(max, s.offsetWidth), 0)
        const recomputeFit = () => {
          const available = observed.clientWidth - 64
          const next = available > 0 && widestPage > 0
            ? Math.min(1, available / widestPage)
            : 1
          setFitScale((cur) => (Math.abs(cur - next) < 0.001 ? cur : next))
        }
        recomputeFit()
        resizeObserver = new ResizeObserver(recomputeFit)
        resizeObserver.observe(observed)
      })
      .catch((err) => {
        if (!cancelled) setError((err as Error).message)
      })
      .finally(() => {
        if (!cancelled) setLoading(false)
      })
    return () => {
      cancelled = true
      resizeObserver?.disconnect()
    }
  }, [format, url])

  // Apply zoom whenever userZoom or the fit scale changes.
  const effectiveZoom = userZoom ?? fitScale
  useEffect(() => {
    if (format !== 'docx') return
    const wrapper = wrapperRef.current
    if (!wrapper) return
    ;(wrapper.style as CSSStyleDeclaration & { zoom?: string }).zoom = String(effectiveZoom)
  }, [format, effectiveZoom])

  useEffect(() => {
    if (format !== 'xlsx') return
    let cancelled = false
    setLoading(true)
    setError(null)
    fetch(url)
      .then((res) => {
        if (!res.ok) throw new Error(`Failed to fetch (${res.status})`)
        return res.arrayBuffer()
      })
      .then((buf) => {
        if (cancelled) return
        const wb = XLSX.read(buf, { type: 'array' })
        const parsed: XlsxSheet[] = wb.SheetNames.map((name) => ({
          name,
          html: XLSX.utils.sheet_to_html(wb.Sheets[name]),
        }))
        setSheets(parsed)
        setActiveSheet(parsed[0]?.name ?? null)
      })
      .catch((err) => {
        if (!cancelled) setError((err as Error).message)
      })
      .finally(() => {
        if (!cancelled) setLoading(false)
      })
    return () => {
      cancelled = true
    }
  }, [format, url])

  const containerStyle: React.CSSProperties = {
    height,
    border: '1px solid var(--mantine-color-gray-3)',
    borderRadius: 6,
    overflow: 'auto',
    background: '#fff',
  }

  if (format === 'pdf') {
    return (
      <iframe
        src={url}
        style={{ width: '100%', height, border: '1px solid var(--mantine-color-gray-3)', borderRadius: 6 }}
        title={fileName}
      />
    )
  }

  if (format === 'image') {
    return (
      <div style={{ ...containerStyle, display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
        <img src={url} alt={fileName} style={{ maxWidth: '100%', maxHeight: '100%' }} />
      </div>
    )
  }

  if (format === 'unsupported') {
    return (
      <div style={{ ...containerStyle, display: 'flex', alignItems: 'center', justifyContent: 'center', padding: 24 }}>
        <Text size="sm">Preview is not available for this file type.</Text>
      </div>
    )
  }

  if (format === 'xlsx') {
    return (
      <div style={{ ...containerStyle, overflow: 'hidden', display: 'flex', flexDirection: 'column' }}>
        {loading && (
          <div style={{ flex: 1, display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
            <Loader size="sm" />
          </div>
        )}
        {error && !loading && (
          <div style={{ padding: 16 }}>
            <Text size="sm" c="red">Failed to render: {error}</Text>
          </div>
        )}
        {!loading && !error && sheets.length > 0 && (
          <Tabs
            value={activeSheet}
            onChange={setActiveSheet}
            keepMounted={false}
            inverted
            color="green"
            classNames={{ tab: 'xlsx-sheet-tab' }}
            style={{ flex: 1, display: 'flex', flexDirection: 'column', minHeight: 0 }}
          >
            {sheets.map((s) => (
              <Tabs.Panel key={s.name} value={s.name} style={{ flex: 1, minHeight: 0, overflow: 'auto' }}>
                <div className="xlsx-sheet" dangerouslySetInnerHTML={{ __html: s.html }} />
              </Tabs.Panel>
            ))}
            <Tabs.List
              style={{
                flexShrink: 0,
                overflowX: 'auto',
                flexWrap: 'nowrap',
                background: 'var(--mantine-color-gray-2)',
                borderTop: '1px solid var(--mantine-color-gray-4)',
                paddingLeft: 4,
              }}
            >
              {sheets.map((s) => (
                <Tabs.Tab
                  key={s.name}
                  value={s.name}
                  style={{
                    whiteSpace: 'nowrap',
                    background: activeSheet === s.name ? '#fff' : 'transparent',
                    fontWeight: activeSheet === s.name ? 600 : 400,
                  }}
                >
                  {s.name}
                </Tabs.Tab>
              ))}
            </Tabs.List>
          </Tabs>
        )}
        <style>{`
          .xlsx-sheet table { border-collapse: collapse; font-size: 12px; }
          .xlsx-sheet td, .xlsx-sheet th {
            border: 1px solid var(--mantine-color-gray-3);
            padding: 4px 8px;
            white-space: nowrap;
          }
          .xlsx-sheet tr:first-child td { background: var(--mantine-color-gray-0); font-weight: 600; }
          .xlsx-sheet-tab {
            border-top-color: transparent !important;
            border-bottom: 4px solid transparent !important;
          }
          .xlsx-sheet-tab[data-active] {
            border-top-color: transparent !important;
            border-bottom-color: var(--mantine-color-green-6) !important;
          }
        `}</style>
      </div>
    )
  }

  const zoomIn = () => setUserZoom((z) => clampZoom((z ?? fitScale) + ZOOM_STEP))
  const zoomOut = () => setUserZoom((z) => clampZoom((z ?? fitScale) - ZOOM_STEP))
  const resetZoom = () => setUserZoom(null)
  const zoomTo100 = () => setUserZoom(1)

  return (
    <div style={{ ...containerStyle, display: 'flex', flexDirection: 'column', overflow: 'hidden' }}>
      {!error && !loading && (
        <Group
          gap={4}
          justify="flex-end"
          style={{
            flexShrink: 0,
            padding: '4px 8px',
            background: 'var(--mantine-color-gray-1)',
            borderBottom: '1px solid var(--mantine-color-gray-3)',
          }}
        >
          <Tooltip label="Fit to width">
            <ActionIcon variant="subtle" size="sm" onClick={resetZoom} aria-label="Fit to width">
              <IconArrowsMaximize size={14} />
            </ActionIcon>
          </Tooltip>
          <Tooltip label="Zoom out">
            <ActionIcon variant="subtle" size="sm" onClick={zoomOut} aria-label="Zoom out">
              <IconMinus size={14} />
            </ActionIcon>
          </Tooltip>
          <Tooltip label="Reset to 100%">
            <ActionIcon variant="subtle" size="sm" onClick={zoomTo100} aria-label="Reset to 100%">
              <IconRefresh size={14} />
            </ActionIcon>
          </Tooltip>
          <Text size="xs" style={{ width: 48, textAlign: 'center', fontVariantNumeric: 'tabular-nums' }}>
            {Math.round(effectiveZoom * 100)}%
          </Text>
          <Tooltip label="Zoom in">
            <ActionIcon variant="subtle" size="sm" onClick={zoomIn} aria-label="Zoom in">
              <IconPlus size={14} />
            </ActionIcon>
          </Tooltip>
        </Group>
      )}
      <div style={{ flex: 1, overflow: 'auto', minHeight: 0 }}>
        {loading && (
          <div style={{ height: '100%', display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
            <Loader size="sm" />
          </div>
        )}
        {error && !loading && (
          <div style={{ padding: 16 }}>
            <Text size="sm" c="red">Failed to render: {error}</Text>
          </div>
        )}
        {!error && (
          <div
            ref={docxRef}
            className="docx-preview-host"
            style={{ visibility: loading ? 'hidden' : 'visible' }}
          />
        )}
      </div>
      <style>{`
        .docx-preview-host {
          background: var(--mantine-color-gray-2);
          min-height: 100%;
          padding: 12px 24px;
          box-sizing: border-box;
          width: max-content;
          min-width: 100%;
        }
        .docx-preview-host .docx-wrapper {
          background: transparent;
          padding: 0;
        }
        .docx-preview-host .docx-wrapper > section.docx {
          margin: 0 auto 16px;
          box-shadow: 0 4px 12px rgba(0, 0, 0, 0.15), 0 1px 3px rgba(0, 0, 0, 0.1);
          background: #fff;
          box-sizing: border-box;
        }
        .docx-preview-host .docx-wrapper > section.docx:last-child {
          margin-bottom: 0;
        }
        .docx-preview-host img { max-width: 100%; height: auto; }
        .docx-preview-host p { line-height: 1.15; }
      `}</style>
    </div>
  )
}

function clampZoom(z: number): number {
  return Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, Math.round(z * 100) / 100))
}
