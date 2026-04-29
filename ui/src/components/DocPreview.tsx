import { useEffect, useRef, useState } from 'react'
import { Loader, Tabs, Text } from '@mantine/core'
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

export function DocPreview({ url, fileName, height = 500 }: Props) {
  const format = detectFormat(fileName)
  const docxRef = useRef<HTMLDivElement | null>(null)
  const [loading, setLoading] = useState(format === 'docx' || format === 'xlsx')
  const [error, setError] = useState<string | null>(null)
  const [sheets, setSheets] = useState<XlsxSheet[]>([])
  const [activeSheet, setActiveSheet] = useState<string | null>(null)

  useEffect(() => {
    if (format !== 'docx') return
    let cancelled = false
    let resizeObserver: ResizeObserver | null = null
    setLoading(true)
    setError(null)
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
        if (!wrapper) return
        const observed = host.parentElement ?? host
        // Look at any descendant that carries an explicit width (sections, article pages, etc.)
        // and pick the widest natural width. Fall back to scrollWidth.
        const widthCandidates = Array.from(
          wrapper.querySelectorAll<HTMLElement>('section.docx, article, .docx-page'),
        )
        const widestPage = Math.max(
          wrapper.scrollWidth,
          ...widthCandidates.map((el) => el.offsetWidth),
        )
        const fit = () => {
          ;(wrapper.style as CSSStyleDeclaration & { zoom?: string }).zoom = '1'
          const available = observed.clientWidth - 48 // gray "desk" side padding
          if (available <= 0 || widestPage <= 0) return
          const scale = Math.min(1, available / widestPage)
          ;(wrapper.style as CSSStyleDeclaration & { zoom?: string }).zoom = String(scale)
        }
        fit()
        resizeObserver = new ResizeObserver(fit)
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

  return (
    <div style={containerStyle}>
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
          style={{ padding: 0, display: loading ? 'none' : 'block' }}
        />
      )}
      <style>{`
        .docx-preview-host {
          background: var(--mantine-color-gray-2);
          min-height: 100%;
          padding: 12px 24px;
          box-sizing: border-box;
          overflow: hidden;
        }
        .docx-preview-host .docx-wrapper {
          background: transparent;
          padding: 0;
        }
        .docx-preview-host .docx-wrapper > section.docx {
          margin: 0 auto 16px;
          box-shadow: 0 4px 12px rgba(0, 0, 0, 0.15), 0 1px 3px rgba(0, 0, 0, 0.1);
          background: #fff;
        }
        .docx-preview-host .docx-wrapper > section.docx:last-child {
          margin-bottom: 0;
        }
        .docx-preview-host img { max-width: 100%; height: auto; }
      `}</style>
    </div>
  )
}
