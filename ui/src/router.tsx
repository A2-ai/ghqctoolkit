import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { createRouter } from '@tanstack/react-router'
import { routeTree } from './routeTree.gen'
import { ROUTER_BASE } from './config'

export function getRouter() {
  const queryClient = new QueryClient({
    defaultOptions: {
      queries: {
        staleTime: 5 * 60 * 1000, // treat fetched data as fresh for 5 minutes
      },
    },
  })

  const router = createRouter({
    routeTree,
    basepath: ROUTER_BASE,
    context: { queryClient },
    defaultPreload: 'intent',
    Wrap: ({ children }) => (
      <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
    ),
  })

  // TanStack Start's hydrateStart() calls router.update({ basepath: TSS_ROUTER_BASEPATH })
  // which is a build-time constant (undefined → '/') and would override our runtime ROUTER_BASE.
  // Wrap update in the browser only so the client keeps ROUTER_BASE while the server
  // continues to reset basepath to '/' for correct server-side route matching.
  // In dev mode, import.meta.url points to source files and gives wrong ROUTER_BASE.
  if (typeof window !== 'undefined') {
    const _update = router.update.bind(router)
    router.update = (opts) => _update({ ...opts, basepath: ROUTER_BASE })
  }

  return router
}

declare module '@tanstack/react-router' {
  interface Register {
    router: ReturnType<typeof getRouter>
  }
}
