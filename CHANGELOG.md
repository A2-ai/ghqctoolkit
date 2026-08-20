# v0.8.0 - Unreleased
## Breaking Changes
* `--ipv4-only` is removed from `ghqc serve` and `ghqc ui`; there is no deprecated alias, so scripts and wrappers still passing the flag will fail with an unknown-flag error and should simply drop it, since the new default is equivalent
* `--bind <ADDR>` (env var `GHQC_BIND`) replaces `--ipv4-only` on `ghqc serve` and `ghqc ui`, and accepts any IP address — `127.0.0.1` or `::1` for loopback only, `0.0.0.0` for every IPv4 interface (needed in containers), `::` for a dual-stack wildcard, or a specific interface address; bracketed IPv6 forms (`[::]`, `[::1]`) are accepted as well, so an address copied out of a printed URL can be pasted straight back into `--bind`
* The server now binds `127.0.0.1` by default instead of the dual-stack wildcard, so an unauthenticated API with permissive CORS is no longer exposed on every interface of the host; the requested address is bound exactly as given, with no IPv6 probing and no fallback to IPv4 on bind failure

## New Features
* `ghqc configuration edit [checklists|options|logo|record]` command to run a single step of the wizard against an existing configuration repository, located from `--config-dir`, the current directory, or the configured configuration directory; with no component named, they are offered as a menu
* `ghqc configuration init` command to interactively create a configuration repository — every `options.yaml` option, copying in a logo, the record template, and authoring markdown checklists in an editor, item by item, or from a bundled starter. Logo and template paths are picked with Tab-completing filesystem browsing. Re-running against an existing configuration repository turns it into an editor: current values become the prompt defaults, and existing checklists can be edited, renamed, or deleted. Files only; committing and pushing is left to the user
* Markdown checklists (`.md`, `.markdown`) are now loaded from the configuration repository, alongside the existing `.txt` and `.yaml`/`.yml` formats; the title comes from the filename and the file content is used as the checklist body verbatim
* `ghqc configuration update` command to fast-forward the local configuration repository to its remote; refuses and leaves the repository untouched when there are uncommitted changes, local commits not on the remote, or diverged history
* `ghqc configuration path` command to print just the configuration repository directory, for use in shells (`cd $(ghqc configuration path)`)
* POST /api/configuration/update endpoint backing the Web UI's configuration update button
* `allow_ui_config_update` option (env var `GHQC_ALLOW_CONFIG_UPDATE`, default `true`) to disable configuration repository updates from the Web UI in deployments where the configuration repository is managed centrally; the `ghqc configuration update` CLI command is deliberately unaffected

## Improvements
* Configuration repository status in the API response now includes `status_detail`, `ahead_commits`, and `behind_commits` on `ConfigGitRepository`
* Configuration tab shows the configuration repository's git status in an always-visible strip with an Update button, enabled when the repository is behind its remote
* Configuration tab is marked with a warning badge and explanatory tooltip when the configuration repository is behind or has diverged, so stale configuration is easier to notice
* Configuration API response `options` now includes the resolved `allow_config_update` boolean, and POST /api/configuration/update returns 403 when updates are disabled
* `ghqc sitrep` and the Web UI's Configuration tab now show every configuration option, including `include_collaborators` and the resolved UI refresh rate and UI config-update toggle, so an administrator can see why the Update button is hidden; `ghqc sitrep --json` gains `resolved_ui_repo_refresh_rate_seconds` and `resolved_allow_ui_config_update`

# v0.7.1 - May 27, 2026
## Improvements
* Git operations now shell out to the system `git` CLI consistently, replacing the previous hybrid approach that mixed the `gix` Rust library with shell-out calls; reduces internal complexity and eliminates `gix` revision-walk usage for commit history

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
