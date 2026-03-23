import { createFileRoute } from '@tanstack/react-router'
import { RecordTab } from '~/components/RecordTab'

export const Route = createFileRoute('/record')({
  component: RecordTab,
})
