import { createContext, useContext, useMemo, useState } from 'react'
import type { ReactNode } from 'react'
import type { ChecklistDraft } from '~/components/ChecklistTab'
import type { CreateOutcome } from '~/components/CreateResultModal'
import type { QueuedItem, RelevantFileDraft } from '~/components/CreateIssueModal'
import type { FileResolution } from '~/components/FileResolveModal'
import type { FilePreviewKind } from '~/api/preview'

export type CreateMilestoneMode = 'select' | 'new'

export interface StatusUiState {
  selectedMilestones: number[]
  includeClosedIssues: Record<number, boolean>
  navWidth: number
  navCollapsed: boolean
}

export interface CreateIssueModalUiState {
  selectedFile: string | null
  checklistDraft: ChecklistDraft
  checklistSelected: boolean
  checklistKey: number
  savedCustomTabs: ChecklistDraft[]
  assignees: string[]
  collaborators: string[]
  collaboratorAuthor: string | null
  collaboratorsSourceFile: string | null
  relevantFiles: RelevantFileDraft[]
  activeTab: string | null
  filePreviewOpen: boolean
  filePreviewMode: FilePreviewKind | 'missing'
  filePreviewContent: string | null
  issuePreviewOpen: boolean
  issuePreviewHtml: string | null
}

export interface CreateUiState {
  mode: CreateMilestoneMode
  selectedMilestone: number | null
  newName: string
  newDesc: string
  modalOpen: boolean
  editingIndex: number | null
  queuedItems: QueuedItem[]
  isCreating: boolean
  createOutcome: CreateOutcome | null
  resultOpen: boolean
  batchOpen: boolean
  modal: CreateIssueModalUiState
}

export type RecordContextItem =
  | { id: string; type: 'file'; serverPath: string; displayName: string }
  | { id: 'qc-record'; type: 'qc-record' }

export interface RecordUiState {
  selectedMilestones: number[]
  showOpenMilestones: boolean
  tablesOnly: boolean
  outputPath: string
  outputPathUserEdited: boolean
  outputPathIsCustom: boolean
  contextItems: RecordContextItem[]
  previewKey: string | null
  previewLoading: boolean
  previewError: string | null
  previewRetryCounter: number
  generateLoading: boolean
  generateError: string | null
  generateSuccess: boolean
  addModalOpen: boolean
  fileTreeKey: number
  milestoneCollapsed: boolean
  rsCollapsed: boolean
  rsHeight: number
  outputHeight: number | null
}

export interface ArchiveUiState {
  selectedMilestones: number[]
  showOpenMilestones: boolean
  includeNonApproved: Record<number, boolean>
  outputPath: string
  outputPathUserEdited: boolean
  outputPathIsCustom: boolean
  flatten: boolean
  generateLoading: boolean
  generateError: string | null
  generateSuccess: string | null
  addedFiles: Map<string, FileResolution>
  editFileModal: string | null
  addFileModalOpen: boolean
}

interface UiSessionValue {
  status: StatusUiState
  setStatus: React.Dispatch<React.SetStateAction<StatusUiState>>
  create: CreateUiState
  setCreate: React.Dispatch<React.SetStateAction<CreateUiState>>
  record: RecordUiState
  setRecord: React.Dispatch<React.SetStateAction<RecordUiState>>
  archive: ArchiveUiState
  setArchive: React.Dispatch<React.SetStateAction<ArchiveUiState>>
}

const defaultCreateModalState: CreateIssueModalUiState = {
  selectedFile: null,
  checklistDraft: { name: '', content: '' },
  checklistSelected: false,
  checklistKey: 0,
  savedCustomTabs: [],
  assignees: [],
  collaborators: [],
  collaboratorAuthor: null,
  collaboratorsSourceFile: null,
  relevantFiles: [],
  activeTab: 'file',
  filePreviewOpen: false,
  filePreviewMode: 'text',
  filePreviewContent: null,
  issuePreviewOpen: false,
  issuePreviewHtml: null,
}

function createModalStateForRouteChange(prev: CreateUiState): CreateIssueModalUiState {
  return {
    ...getDefaultCreateModalState(),
    savedCustomTabs: prev.modal.savedCustomTabs,
    checklistKey: prev.modal.checklistKey,
  }
}

const UiSessionContext = createContext<UiSessionValue | null>(null)
const defaultRecordState: RecordUiState = {
  selectedMilestones: [],
  showOpenMilestones: false,
  tablesOnly: false,
  outputPath: '',
  outputPathUserEdited: false,
  outputPathIsCustom: false,
  contextItems: [{ id: 'qc-record', type: 'qc-record' }],
  previewKey: null,
  previewLoading: false,
  previewError: null,
  previewRetryCounter: 0,
  generateLoading: false,
  generateError: null,
  generateSuccess: false,
  addModalOpen: false,
  fileTreeKey: 0,
  milestoneCollapsed: false,
  rsCollapsed: false,
  rsHeight: 300,
  outputHeight: null,
}

const defaultArchiveState: ArchiveUiState = {
  selectedMilestones: [],
  showOpenMilestones: false,
  includeNonApproved: {},
  outputPath: '',
  outputPathUserEdited: false,
  outputPathIsCustom: false,
  flatten: false,
  generateLoading: false,
  generateError: null,
  generateSuccess: null,
  addedFiles: new Map(),
  editFileModal: null,
  addFileModalOpen: false,
}

export function UiSessionProvider({ children }: { children: ReactNode }) {
  const [status, setStatus] = useState<StatusUiState>({
    selectedMilestones: [],
    includeClosedIssues: {},
    navWidth: 320,
    navCollapsed: false,
  })

  const [create, setCreate] = useState<CreateUiState>({
    mode: 'select',
    selectedMilestone: null,
    newName: '',
    newDesc: '',
    modalOpen: false,
    editingIndex: null,
    queuedItems: [],
    isCreating: false,
    createOutcome: null,
    resultOpen: false,
    batchOpen: false,
    modal: defaultCreateModalState,
  })

  const [record, setRecord] = useState<RecordUiState>(defaultRecordState)
  const [archive, setArchive] = useState<ArchiveUiState>(defaultArchiveState)

  const value = useMemo(
    () => ({ status, setStatus, create, setCreate, record, setRecord, archive, setArchive }),
    [status, create, record, archive],
  )

  return <UiSessionContext.Provider value={value}>{children}</UiSessionContext.Provider>
}

export function useUiSession() {
  const value = useContext(UiSessionContext)
  if (!value) throw new Error('useUiSession must be used within UiSessionProvider')
  return value
}

export function getDefaultCreateModalState(): CreateIssueModalUiState {
  return {
    selectedFile: null,
    checklistDraft: { name: '', content: '' },
    checklistSelected: false,
    checklistKey: 0,
    savedCustomTabs: [],
    assignees: [],
    collaborators: [],
    collaboratorAuthor: null,
    collaboratorsSourceFile: null,
    relevantFiles: [],
    activeTab: 'file',
    filePreviewOpen: false,
    filePreviewMode: 'text',
    filePreviewContent: null,
    issuePreviewOpen: false,
    issuePreviewHtml: null,
  }
}

export function closeCreateModals(prev: CreateUiState): CreateUiState {
  return {
    ...prev,
    modalOpen: false,
    editingIndex: null,
    createOutcome: null,
    resultOpen: false,
    batchOpen: false,
    modal: createModalStateForRouteChange(prev),
  }
}

export function closeRecordModals(prev: RecordUiState): RecordUiState {
  return {
    ...prev,
    previewKey: null,
    previewLoading: false,
    previewError: null,
    addModalOpen: false,
  }
}

export function closeArchiveModals(prev: ArchiveUiState): ArchiveUiState {
  return {
    ...prev,
    editFileModal: null,
    addFileModalOpen: false,
  }
}
