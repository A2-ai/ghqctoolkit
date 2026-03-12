# Serving a TanStack Start SPA Behind Posit Workbench

## The Problem

Posit Workbench proxies locally-running apps to a URL of the form:

```
https://<host>/rstudio/s/<session-id>/p/<port-id>/
```

Both `<session-id>` and `<port-id>` are opaque hashes unknown at build time. The app
receives requests with the prefix already stripped by the proxy — it only sees `/` — but
the browser's address bar shows the full prefixed URL.

This creates two classes of failures for a TanStack Start SPA:

1. **Asset loading** — HTML like `<script src="/assets/main.js">` resolves to
   `https://<host>/assets/main.js` (root-relative), bypassing the proxy prefix entirely.
   The Workbench proxy returns 404.

2. **Client-side routing** — TanStack Router's `basepath` must match the prefix visible
   in the browser or route matching fails with an invariant error.

---

## Solution Overview

The fix has four parts, each targeting a specific failure mode:

| File | Change | Fixes |
|---|---|---|
| `ui/vite.config.ts` | `base: ''` | Client-side lazy-chunk imports become relative |
| `ui/src/server.ts` | `transformAssetUrls: '.'` | SSR manifest links become `./assets/...` |
| `ui/src/router.tsx` | Wrap `router.update` | Preserves runtime `basepath` against TSS override |
| `ui/src/components/AppLayout.tsx` + `__root.tsx` | `./logo.png` | Public static assets use relative paths |

---

## Part 1 — `base: ''` in Vite

```ts
// ui/vite.config.ts
export default defineConfig({
  base: '',
  // ...
})
```

With the default `base: '/'`, Vite bakes absolute paths into every client-side dynamic
`import()` call (e.g. `import('/assets/chunk-route.js')`). Once the entry module is loaded
from `https://<host>/rstudio/.../assets/main.js`, those absolute imports miss the proxy
prefix.

`base: ''` makes Vite generate **relative** import paths (`import('./chunk-route.js')`).
These resolve relative to wherever the entry module was loaded from, so they
automatically inherit the correct proxy prefix at runtime.

> **Why `''` and not `'./'`?**
> `base: './'` produces broken paths in the SSR manifest like `/./assets/...`.
> `base: ''` keeps SSR manifest paths as root-relative `/assets/...` while still
> generating relative client-side imports.

---

## Part 2 — `transformAssetUrls` in the Server Entry

TanStack Start's SSR manifest controls the `<link rel="modulepreload">`,
`<link rel="stylesheet">`, and `<script>` tags injected into the initial HTML. By default
these use root-relative paths (`/assets/...`), which break under the proxy.

Create `ui/src/server.ts` to override the default server entry:

```ts
// ui/src/server.ts
import {
  createStartHandler,
  defaultStreamHandler,
} from '@tanstack/react-start/server'
import type { Register } from '@tanstack/react-router'
import type { RequestHandler } from '@tanstack/react-start/server'

// Prepend '.' so /assets/foo.js → ./assets/foo.js in the emitted HTML.
// The browser resolves ./assets/foo.js relative to the page URL, which already
// includes the full proxy prefix — no server-side knowledge of the prefix needed.
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
```

The TanStack Start Vite plugin automatically picks up `src/server.ts` as the server entry
when the file exists, overriding the default.

`transformAssetUrls: '.'` is a static string prefix. Combined with `base: ''` preserving
root-relative paths in the SSR manifest, this produces:

```
/assets/main.js  →  ./assets/main.js
```

No per-request header reading is needed. The relative URL resolves correctly at any proxy
prefix.

---

## Part 3 — Runtime `basepath` for TanStack Router

TanStack Router needs its `basepath` set to the full proxy prefix so that client-side
navigation works correctly. Since the prefix is unknown at build time, it is derived from
`import.meta.url` at runtime.

### Deriving the base from the module URL

```ts
// ui/src/config.ts
const appRoot = new URL('..', import.meta.url)
export const API_BASE = new URL('api', appRoot).href
export const ROUTER_BASE = appRoot.pathname.replace(/\/$/, '') || '/'
```

In a production build, `import.meta.url` is the URL of the loaded bundle, e.g.
`https://<host>/rstudio/s/.../p/.../assets/main.js`. `new URL('..', ...)` walks up one
directory to the app root, giving `ROUTER_BASE = '/rstudio/s/.../p/...'`.

### The TanStack Start override problem

TanStack Start's `hydrateStart()` calls `router.update({ basepath: process.env.TSS_ROUTER_BASEPATH })` during client startup. `TSS_ROUTER_BASEPATH` is a build-time constant injected by the plugin — it defaults to `undefined`, which causes the router to reset `basepath` to `'/'`. This overwrites the runtime-computed `ROUTER_BASE`.

### The fix — wrap `router.update`

```ts
// ui/src/router.tsx  (inside getRouter())

const router = createRouter({
  routeTree,
  basepath: ROUTER_BASE,
  // ...
})

// Only in production: dev mode derives ROUTER_BASE from source file paths,
// which gives wrong values (e.g. '/src'). In dev, let TSS reset to '/' normally.
if (typeof window !== 'undefined' && !import.meta.env.DEV) {
  const _update = router.update.bind(router)
  router.update = (opts) => _update({ ...opts, basepath: ROUTER_BASE })
}
```

This ensures:
- **Production / Workbench proxy:** `ROUTER_BASE` persists through TanStack Start's override.
- **Dev mode (`bun run dev`):** The wrapper is skipped; TanStack Start resets to `'/'` as normal.
- **Server-side (gen-index-html):** `typeof window === 'undefined'`, so the wrapper never runs; the server correctly resets to `'/'` for route matching against build-time requests.

---

## Part 4 — Public Static Assets

Assets in `public/` (like `logo.png`) are not processed by Vite or `transformAssetUrls`.
Any hardcoded `/logo.png` path in JSX or in `<link>` tags will break under the proxy.

Change all such references to `./logo.png`:

```tsx
// ui/src/routes/__root.tsx
{ rel: 'icon', type: 'image/png', href: './logo.png' }

// ui/src/components/AppLayout.tsx
<img src="./logo.png" alt="ghqc logo" />
```

---

## Part 5 — Simplify the Rust Server

The Rust server previously patched HTML at runtime in `serve_index` to work around the
absolute asset paths. With all paths now relative in the generated `index.html`, this
is no longer needed.

**Before:**
```rust
fn serve_index(path: &str) -> Response {
    let html = String::from_utf8_lossy(&index.data);
    let html = html.replace("\"/assets/", "\"./assets/");
    let html = html.replace("href=\"/logo.", "href=\"./logo.");
    let html = html.replacen("<head>", &format!("<head><base href=\"{base_href}\">"), 1);
    // ...
}
```

**After:**
```rust
fn serve_index() -> Response {
    match UiAssets::get("index.html") {
        Some(index) => Response::builder()
            .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
            .header(header::CONTENT_LENGTH, index.data.len())
            .body(Body::from(index.data))
            .unwrap_or(/* ... */),
        None => (StatusCode::NOT_FOUND, "404 Not Found").into_response(),
    }
}
```

---

## How It All Fits Together

```
Browser at: https://<host>/rstudio/s/<sid>/p/<pid>/
                │
                ├─ Loads ./assets/main.js          ← relative, via transformAssetUrls
                │    └─ import.meta.url = https://<host>/rstudio/.../assets/main.js
                │
                ├─ config.ts derives ROUTER_BASE = /rstudio/s/<sid>/p/<pid>
                │    └─ router.update wrapper locks this in against TSS override
                │
                ├─ Lazy route chunks: import('./chunk.js')  ← relative, via base: ''
                │    └─ resolves from assets/ → correct proxy URL
                │
                └─ API calls: API_BASE = https://<host>/rstudio/.../api
                     └─ proxy strips prefix → Rust server sees /api/...
```

---

## Caveats

- `import.meta.url` only gives the correct proxy prefix in **production builds**. In Vite
  dev mode it points to source files, so `ROUTER_BASE` would be wrong (e.g. `/src`). The
  `!import.meta.env.DEV` guard in `router.tsx` handles this.

- `transformAssetUrls` is marked **experimental** in TanStack Start and may change in
  future versions.

- This approach does not require any Posit Workbench-specific headers or configuration.
  It works purely through relative URL resolution in the browser.
