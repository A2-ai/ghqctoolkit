import { createFileRoute } from '@tanstack/react-router'
import { CreateTab } from '~/components/CreateTab'

export const Route = createFileRoute('/create')({
  component: CreateTab,
})
