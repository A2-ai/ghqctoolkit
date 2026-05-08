# v0.7.0 - May 8, 2026
## Action Required
**After installing this release, clear the commit cache with `ghqc cache remove commits --global`** (see the new `ghqc cache` command below).

Previous releases used `git log --full-history` to determine file-changing commits, which incorrectly flagged merge commits that touched the file on another branch without changing it on the QC branch. That's fixed in 0.7.0, but cached results from prior versions will still reflect the old behavior until cleared.

## New Features
* Markdown editor for review comments, checklist editing, and issue detail previews
* Word (.docx) and Excel (.xlsx) file previews in the Create and Archive tabs, rendered via in-browser viewers
* `ghqc cache` command to provide insight and remove entries

## Improvements
* Checklist state now persists across tab switches and reloads
* Status tab tooltips clarify status colors and the approve-comment lock state
* Better error messaging and suggestions for non-local branches, with improved branch error handling
* "Ready for Review" issues now default to the Review tab unless checklist and relevant files are incomplete
* Checklist column is expandable and shows a tooltip with the full checklist name
* Milestone ordering reverted to prior behavior; tab completion improved; input is trimmed of extra whitespace
* git log walks no longer use `--full-history`, improving performance on file-changing commits
* `octocrab` updated to support the "Closed as Duplicate" issue status
* Config directory resolution strips a trailing `.git` from the repo name
* Auth store not-found log downgraded from warning to debug to reduce user confusion

# v0.6.0 - April 9, 2026
## New Features
* File rename tracking: after issue creation, the UI detects when the associated file has been renamed and prompts to update the issue link; a `ghqc rename` CLI command is also available

## Improvements
* GitHub comment body splitting: issue bodies and review comments that exceed GitHub's character limit are automatically split into multiple comments
* Commit history in the issue detail view is now scrollable, with the most recent commit shown by default

# v0.5.0 - April 7, 2026
## New Features
* File preview in the Create and Archive tabs: text, PDF, and Word files can be previewed inline; unsupported file types show a descriptive message
* Previous QC diff comments can now be previewed before posting in the Relevant Files picker

## Improvements
* Typst record formatting now correctly renders markdown links, inline code spans, and bare URLs
* Commit search performance improved via disk-backed caching, replacing the in-memory cache
* Status tab: horizontal scrollbar for wide boards; issues in `approval_required` state are now highlighted red
* Cache writes use atomic file replacement to prevent race conditions

# v0.4.1 - April 2, 2026
## Improvements
* Web UI's status tab has individually scrollable swimlanes
* `--skip-gh` on `gh auth login` to skip using the `gh` CLI if found

# v0.4.0 - April 2, 2026
## New Features
* Configurable issue collaborators in both the CLI and Web UI
* Configuration status API/UI support for surfacing the active repository options
* `ghqc ui url` command for retrieving the local Web UI address
* `ghqc auth token` command for retrieving the auth token that will be used

## Improvements
* Review posting can now opt out of auto-stashing local changes
* Issue creation, preview, and record flows now refresh authentication and repository state more reliably
* Web UI repository refresh interval is now configurable
* Server startup now supports variable socket binding, random port assignment, and `--ipv4-only`
* Install scripts now support installing a specific released version
* Authentication handling improved for non-GitHub environments
* Typst-backed record output formatting improved

## Patches
* Fixed blocking QC API request behavior during refresh-heavy workflows
* Issue preview and detail views now better preserve sizing and collaborator state

# v0.3.0 - March 24, 2026
## New Features
* `ghqc auth login`, `ghqc auth logout`, and `ghqc auth status` commands for managing GitHub authentication
* Windows PowerShell installer for downloading and installing the latest release

## Improvements
* "Previous QC" references can now post an automatic diff comment
* `ghqc sitrep` now reports authentication store and available auth sources
* Web UI now supports direct routes for each tab
* States persists across UI tab switches

## Patches
* Server bind/listen behavior updated for IPv6 compatibility

# v0.2.1 - March 12, 2026
## New Features
* `GHQC_CONFIG_DIR` env var to set a config directory fallback (for share team configuration)

## Improvements
* Config directory validation for non-git directory
* Approval/unapproval cascades to related issues
* Issue status updates on repo's HEAD commit changes
* Posit Workbench / RStudio server proxy support

## Patches
* Checklist save functionality
* Archive directory path check relative to server directory

# v0.2.0 - March 5, 2026
## New Features
* Sitrep - Introduced the `ghqc sitrep` command to return current repository status report

## Improvements
* In POST /api/milestones/{number}/issues, create issue labels if needed before issue creation

# v0.1.0 - March 4, 2026
Initial Release
