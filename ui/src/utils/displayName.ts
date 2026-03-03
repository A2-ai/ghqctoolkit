/**
 * Given a raw checklist_display_name (e.g. "checklist", "checklists", "Code Review"),
 * return singular and plural forms by stripping/adding a trailing 's'.
 *
 * Both "checklist" and "checklists" produce the same result:
 *   { singular: "checklist", plural: "checklists" }
 */
export function resolveDisplayName(raw: string): { singular: string; plural: string } {
  const singular = raw.toLowerCase().endsWith('s') ? raw.slice(0, -1) : raw
  const plural = singular + 's'
  return { singular, plural }
}

/** Capitalize the first character of a string; leaves the rest unchanged. */
export function capitalize(s: string): string {
  if (!s) return s
  return s.charAt(0).toUpperCase() + s.slice(1)
}
