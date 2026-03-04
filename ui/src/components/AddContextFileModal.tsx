import { useRef, useState } from 'react'
import { Alert, Button, Loader, Modal, Stack, Tabs, Text } from '@mantine/core'
import { IconUpload } from '@tabler/icons-react'
import { FileTreeBrowser } from './FileTreeBrowser'
import { uploadContextFile } from '~/api/record'

interface AddItem {
  serverPath: string
  displayName: string
}

interface Props {
  opened: boolean
  onClose: () => void
  onAdd: (item: AddItem) => void
  /** Increment to force the file tree to re-fetch (e.g. after generating a new PDF) */
  fileTreeKey?: number
}

export function AddContextFileModal({ opened, onClose, onAdd, fileTreeKey = 0 }: Props) {
  const [selectedFile, setSelectedFile] = useState<string | null>(null)
  const [uploadError, setUploadError] = useState<string | null>(null)
  const [uploading, setUploading] = useState(false)
  const fileInputRef = useRef<HTMLInputElement>(null)

  function handleBrowseAdd() {
    if (!selectedFile) return
    const displayName = selectedFile.split('/').pop() ?? selectedFile
    onAdd({ serverPath: selectedFile, displayName })
    setSelectedFile(null)
    onClose()
  }

  async function handleFileChange(e: React.ChangeEvent<HTMLInputElement>) {
    const file = e.target.files?.[0]
    if (!file) return
    setUploadError(null)
    setUploading(true)
    try {
      const result = await uploadContextFile(file)
      onAdd({ serverPath: result.temp_path, displayName: file.name })
      onClose()
    } catch (err) {
      setUploadError((err as Error).message)
    } finally {
      setUploading(false)
      // Reset input so the same file can be re-selected
      if (fileInputRef.current) fileInputRef.current.value = ''
    }
  }

  return (
    <Modal opened={opened} onClose={onClose} title="Add Context File" size="lg">
      <Tabs defaultValue="browse">
        <Tabs.List>
          <Tabs.Tab value="browse">Browse Server</Tabs.Tab>
          <Tabs.Tab value="upload">Upload</Tabs.Tab>
        </Tabs.List>

        <Tabs.Panel value="browse" pt="md">
          <Stack gap="md">
            <FileTreeBrowser
              key={fileTreeKey}
              selectedFile={selectedFile}
              onSelect={setSelectedFile}
              filterFile={(name) => name.toLowerCase().endsWith('.pdf')}
            />
            <Button
              disabled={!selectedFile}
              onClick={handleBrowseAdd}
              fullWidth
            >
              Add Selected File
            </Button>
          </Stack>
        </Tabs.Panel>

        <Tabs.Panel value="upload" pt="md">
          <Stack gap="md">
            <Text size="sm" c="dimmed">
              Upload a PDF from your local machine. The file will be stored temporarily on the server.
            </Text>

            {uploadError && <Alert color="red">{uploadError}</Alert>}

            {uploading ? (
              <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                <Loader size="sm" />
                <Text size="sm">Uploading…</Text>
              </div>
            ) : (
              <div
                style={{
                  border: '2px dashed var(--mantine-color-gray-4)',
                  borderRadius: 8,
                  padding: '32px 16px',
                  textAlign: 'center',
                  cursor: 'pointer',
                }}
                onClick={() => fileInputRef.current?.click()}
              >
                <IconUpload size={32} color="var(--mantine-color-gray-5)" />
                <Text size="sm" c="dimmed" mt={8}>
                  Click to select a PDF file
                </Text>
                <input
                  ref={fileInputRef}
                  type="file"
                  accept=".pdf,application/pdf"
                  style={{ display: 'none' }}
                  onChange={handleFileChange}
                />
              </div>
            )}
          </Stack>
        </Tabs.Panel>
      </Tabs>
    </Modal>
  )
}
