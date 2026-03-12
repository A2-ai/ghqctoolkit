import {
  createStartHandler,
  defaultStreamHandler,
} from '@tanstack/react-start/server'
import type { Register } from '@tanstack/react-router'
import type { RequestHandler } from '@tanstack/react-start/server'

// Prepend '.' to every manifest asset URL so paths are relative (e.g. /assets/foo.js → ./assets/foo.js).
// This lets the browser resolve assets against the current page URL, which means the app works
// correctly behind a proxy with an unknown prefix (e.g. Posit Workbench /s/<id>/p/<id>/).
const fetch = createStartHandler({
  handler: defaultStreamHandler,
  transformAssetUrls: '.',
})

export type ServerEntry = { fetch: RequestHandler<Register> }

export function createServerEntry(entry: ServerEntry): ServerEntry {
  return {
    async fetch(...args) {
      return await entry.fetch(...args)
    },
  }
}

export default createServerEntry({ fetch })
