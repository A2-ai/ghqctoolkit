import { useEffect, useState } from 'react'
import { Tree } from 'react-arborist'
import { Alert, Checkbox, Loader, Text } from '@mantine/core'
import { IconFolder } from '@tabler/icons-react'
import { fetchFileTree } from '~/api/files'
import type { NodeRendererProps } from 'react-arborist'

interface FileNode {
  id: string
  name: string
  children?: FileNode[] | null
}

interface Props {
  selectedFile: string | null
  onSelect: (file: string | null) => void
  claimedFiles?: Set<string>
}

function injectChildren(
  nodes: FileNode[],
  targetId: string,
  children: FileNode[],
): FileNode[] {
  return nodes.map((node) => {
    if (node.id === targetId) {
      return { ...node, children }
    }
    if (Array.isArray(node.children) && node.children.length > 0) {
      return { ...node, children: injectChildren(node.children, targetId, children) }
    }
    return node
  })
}

function findNode(nodes: FileNode[], targetId: string): FileNode | null {
  for (const node of nodes) {
    if (node.id === targetId) return node
    if (Array.isArray(node.children)) {
      const found = findNode(node.children, targetId)
      if (found) return found
    }
  }
  return null
}

export function FileTreeBrowser({ selectedFile, onSelect, claimedFiles = new Set() }: Props) {
  const [data, setData] = useState<FileNode[]>([])
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)

  function NodeRenderer({ node, style, dragHandle }: NodeRendererProps<FileNode>) {
    const claimed = !node.isInternal && claimedFiles.has(node.id)
    return (
      <div
        style={{
          ...style,
          display: 'flex',
          alignItems: 'center',
          gap: 6,
          cursor: claimed ? 'not-allowed' : 'pointer',
          opacity: claimed ? 0.4 : 1,
        }}
        ref={dragHandle}
        onClick={() => {
          if (node.isInternal) {
            node.toggle()
          } else if (!claimed) {
            onSelect(node.id === selectedFile ? null : node.id)
          }
        }}
      >
        {node.isInternal ? (
          <IconFolder size={14} color={node.isOpen ? '#2f7a3b' : '#e6a817'} style={{ flexShrink: 0 }} />
        ) : (
          <Checkbox
            size="xs"
            checked={node.id === selectedFile}
            readOnly
            style={{ pointerEvents: 'none', flexShrink: 0 }}
          />
        )}
        <Text size="sm" style={{ userSelect: 'none', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
          {node.data.name}
        </Text>
      </div>
    )
  }

  useEffect(() => {
    fetchFileTree('')
      .then((res) => {
        const nodes: FileNode[] = res.entries.map((entry) => ({
          id: entry.name,
          name: entry.name,
          children: entry.kind === 'directory' ? null : undefined,
        }))
        setData(nodes)
        setLoading(false)
      })
      .catch((err: Error) => {
        setError(err.message)
        setLoading(false)
      })
  }, [])

  async function handleToggle(nodeId: string) {
    const node = findNode(data, nodeId)
    // children === null means "directory, not yet fetched" — only load in that case
    if (!node || node.children !== null) return
    try {
      const res = await fetchFileTree(nodeId)
      const children: FileNode[] = res.entries.map((entry) => ({
        id: `${nodeId}/${entry.name}`,
        name: entry.name,
        children: entry.kind === 'directory' ? null : undefined,
      }))
      setData((prev) => injectChildren(prev, nodeId, children))
    } catch {
      // Silently ignore sub-directory load failures
    }
  }

  if (loading) return <Loader size="sm" />
  if (error) return <Alert color="red">{error}</Alert>

  return (
    <Tree<FileNode>
      data={data}
      childrenAccessor={(d) => {
        if (d.children === undefined) return null // leaf (file)
        return d.children ?? [] // null → empty array (unloaded dir), array → loaded
      }}
      onToggle={handleToggle}
      openByDefault={false}
      disableDrag
      disableDrop
      disableEdit
      rowHeight={28}
      width="100%"
      height={360}
    >
      {NodeRenderer}
    </Tree>
  )
}
