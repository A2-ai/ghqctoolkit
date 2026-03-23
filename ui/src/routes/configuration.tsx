import { createFileRoute } from '@tanstack/react-router'
import { ConfigurationTab } from '~/components/ConfigurationTab'

export const Route = createFileRoute('/configuration')({
  component: ConfigurationTab,
})
