import { createFileRoute } from '@tanstack/react-router'
import { ArchiveTab } from '~/components/ArchiveTab'

export const Route = createFileRoute('/archive')({
  component: ArchiveTab,
})
