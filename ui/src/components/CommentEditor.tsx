import type { KeyboardEvent, TextareaHTMLAttributes } from 'react'
import { Input, useMantineColorScheme } from '@mantine/core'
import {
  IconBold,
  IconItalic,
  IconHeading,
  IconQuote,
  IconCode,
  IconSourceCode,
  IconLink,
  IconList,
  IconListNumbers,
  IconListCheck,
} from '@tabler/icons-react'
import MDEditor, { commands } from '@uiw/react-md-editor'
import '@uiw/react-md-editor/markdown-editor.css'

const ICON_SIZE = 16
const ICON_STROKE = 1.75

type Props = {
  label?: string
  placeholder?: string
  value: string
  onChange: (value: string) => void
  required?: boolean
  error?: string | false
  minHeight?: number
  monospace?: boolean
  onKeyDown?: (e: KeyboardEvent<HTMLTextAreaElement>) => void
  textareaProps?: Omit<TextareaHTMLAttributes<HTMLTextAreaElement>, 'value'>
}

const tablerIcon = (Icon: typeof IconBold) => (
  <Icon size={ICON_SIZE} stroke={ICON_STROKE} />
)

const toolbarCommands = [
  { ...commands.title, icon: tablerIcon(IconHeading) },
  { ...commands.bold, icon: tablerIcon(IconBold) },
  { ...commands.italic, icon: tablerIcon(IconItalic) },
  commands.divider,
  { ...commands.quote, icon: tablerIcon(IconQuote) },
  { ...commands.code, icon: tablerIcon(IconCode) },
  { ...commands.codeBlock, icon: tablerIcon(IconSourceCode) },
  commands.divider,
  { ...commands.link, icon: tablerIcon(IconLink) },
  { ...commands.unorderedListCommand, icon: tablerIcon(IconList) },
  { ...commands.orderedListCommand, icon: tablerIcon(IconListNumbers) },
  { ...commands.checkedListCommand, icon: tablerIcon(IconListCheck) },
]

export function CommentEditor({
  label,
  placeholder,
  value,
  onChange,
  required,
  error,
  minHeight = 80,
  monospace,
  onKeyDown,
  textareaProps,
}: Props) {
  const { colorScheme } = useMantineColorScheme()
  const dataColorMode = colorScheme === 'dark' ? 'dark' : 'light'

  return (
    <Input.Wrapper label={label} required={required} error={error || undefined}>
      <div data-color-mode={dataColorMode} style={{ marginTop: label ? 4 : 0 }}>
        <MDEditor
          value={value}
          onChange={(v) => onChange(v ?? '')}
          preview="edit"
          hideToolbar={false}
          visibleDragbar={true}
          commands={toolbarCommands}
          extraCommands={[]}
          height={minHeight + 60}
          textareaProps={
            {
              placeholder,
              onKeyDown,
              ...textareaProps,
              style: monospace
                ? { fontFamily: 'monospace', fontSize: 12, ...(textareaProps?.style ?? {}) }
                : textareaProps?.style,
            } as React.ComponentProps<typeof MDEditor>['textareaProps']
          }
          style={{
            border: error ? '1px solid var(--mantine-color-error)' : '1px solid var(--mantine-color-default-border)',
            borderRadius: 'var(--mantine-radius-sm)',
          }}
        />
      </div>
    </Input.Wrapper>
  )
}
