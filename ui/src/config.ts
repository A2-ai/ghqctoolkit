const APP_ROUTE_SUFFIXES = [
  '/status',
  '/create',
  '/record',
  '/archive',
  '/configuration',
] as const

function trimTrailingSlash(pathname: string): string {
  return pathname.length > 1 && pathname.endsWith('/')
    ? pathname.slice(0, -1)
    : pathname
}

function inferRouterBaseFromPathname(pathname: string): string {
  const normalized = trimTrailingSlash(pathname)

  if (normalized === '/') {
    return '/'
  }

  for (const route of APP_ROUTE_SUFFIXES) {
    if (normalized === route) {
      return '/'
    }

    if (normalized.endsWith(route)) {
      const base = normalized.slice(0, -route.length)
      return base || '/'
    }
  }

  return normalized || '/'
}

function getAppRoot(): URL {
  if (typeof window !== 'undefined') {
    const basePath = inferRouterBaseFromPathname(window.location.pathname)
    return new URL(`${basePath.replace(/\/?$/, '/')}`, window.location.origin)
  }

  return new URL('/', 'http://localhost')
}

const appRoot = getAppRoot()

export const API_BASE = new URL('api', appRoot).href
export const ROUTER_BASE = appRoot.pathname.replace(/\/$/, '') || '/'
