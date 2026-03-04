/**
 * Post-build: calls the TanStack Start server bundle with a request for "/"
 * and writes the resulting HTML to dist/client/index.html so it can be
 * embedded into the Rust binary via rust-embed.
 */
import path from 'node:path'
import { writeFile } from 'node:fs/promises'

const serverPath = path.resolve(process.cwd(), 'dist/server/server.js')

// Dynamic import so we don't fail at parse time if the file doesn't exist
const handler = await import(serverPath)
const req = new Request('http://localhost/')
const resp = await handler.default.fetch(req)
const html = await resp.text()

const outPath = path.resolve(process.cwd(), 'dist/client/index.html')
await writeFile(outPath, html, 'utf8')
console.log(`Generated dist/client/index.html (${html.length} bytes)`)
