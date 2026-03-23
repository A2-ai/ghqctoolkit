import { createFileRoute } from '@tanstack/react-router'
import { StatusTab } from '~/components/StatusTab'

export const Route = createFileRoute('/status')({
  component: StatusTab,
})
