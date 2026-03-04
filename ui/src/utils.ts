export function extractIssueNumber(url: string): number | null {
  const match = url.match(/\/issues\/(\d+)(?:[^/]*)$/)
  return match ? parseInt(match[1], 10) : null
}
