/** Wraps a bare HTML fragment in a GitHub-styled document for iframe srcDoc. */
export function wrapInGithubStyles(body: string): string {
  return `<!DOCTYPE html>
<html>
<head>
<meta charset="utf-8">
<style>
  body {
    font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Helvetica, Arial, sans-serif;
    font-size: 14px; line-height: 1.6; color: #1f2328;
    padding: 16px 20px; margin: 0; word-wrap: break-word;
  }
  h1,h2,h3,h4,h5,h6 { margin-top: 20px; margin-bottom: 8px; font-weight: 600; line-height: 1.25; }
  h2 { padding-bottom: 6px; border-bottom: 1px solid #d0d7de; font-size: 1.3em; }
  h3 { font-size: 1.1em; }
  a { color: #0969da; text-decoration: none; }
  a:hover { text-decoration: underline; }
  p { margin-top: 0; margin-bottom: 12px; }
  ul,ol { padding-left: 2em; margin-top: 0; margin-bottom: 12px; }
  li { margin-bottom: 2px; }
  li:has(> input[type="checkbox"]) { list-style: none; }
  li:has(> input[type="checkbox"]) input[type="checkbox"] { margin: 0 0.3em 0.2em -1.4em; vertical-align: middle; }
  code { font-family: ui-monospace,SFMono-Regular,"SF Mono",Menlo,monospace; font-size: 85%; background: rgba(175,184,193,0.2); padding: 2px 5px; border-radius: 4px; }
  pre { background: #f6f8fa; border-radius: 6px; padding: 12px 16px; overflow: auto; font-size: 85%; line-height: 1.45; }
  pre code { background: none; padding: 0; }
  blockquote { margin: 0 0 12px; padding: 0 12px; color: #57606a; border-left: 4px solid #d0d7de; }
  hr { border: none; border-top: 1px solid #d0d7de; margin: 16px 0; }
  table { border-collapse: collapse; width: 100%; margin-bottom: 12px; }
  th,td { border: 1px solid #d0d7de; padding: 6px 12px; }
  th { background: #f6f8fa; font-weight: 600; }
</style>
</head>
<body>${body}</body>
</html>`
}
