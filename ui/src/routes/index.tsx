import { createFileRoute } from '@tanstack/react-router'

export const Route = createFileRoute('/')({
  component: HomeRoute,
})

function HomeRoute() {
  return <h1>ghqc UI — ready for development</h1>
}
