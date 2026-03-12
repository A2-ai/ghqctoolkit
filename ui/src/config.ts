const appRoot = new URL("..", import.meta.url)
export const API_BASE = new URL('api', appRoot).href
export const ROUTER_BASE = appRoot.pathname.replace(/\/$/, '') || '/'
