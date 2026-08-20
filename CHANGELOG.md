# v0.8.0 - Unreleased
## Action Required
**`ghqc_archive_metadata.json` has a new shape, and the `approved` field is gone.** Anything reading that file — a script, a checklist, a QC report generator — must be updated before reading an archive written by this release. The single `approved: true`/`false` bool is replaced by a `round` object per milestone file: `round.round` (the round the archive selection addressed), `round.approval` (the round that closed on the archived commit, with who approved it and when, or `null` when the bytes were never approved), and `round.superseded` (whether anything newer than those bytes might exist — set whenever those bytes are **not provably current**, including when a later part of the history could not be read *or* spans histories with no common ancestor). The bool could not express the case rounds exist for — approved in an earlier round, under review again now — and it was `true` for every archive taken after the first approval regardless of what happened since, so there is no compatible reading of it to keep. **Archives written before this release can no longer be read back by ghqc: their metadata is refused as an unsupported structure version, with an error naming the version found and the version this build reads.** That refusal is deliberate and it is the safe outcome. A pre-round document does not fail to parse — the QC block is a flattened optional field, so an old file parses *successfully* with the QC block absent, and every QC'd file in it reads back as a manually added file with no milestone, no approval and no QC history at all. A false negative on approval inside an audit artifact is worse than a file that says it cannot be read, so the reader refuses the shape instead of misreporting it. Nothing in ghqc reads archive metadata back, so no existing workflow breaks; the refusal exists so that a future reader cannot silently misread an old archive. The archived *files* in an old archive are of course untouched — it is a tarball, and only `ghqc_archive_metadata.json` is affected.

## New Features
* `ghqc_archive_metadata.json` now carries `metadata_version`, serialized first so `head`ing the file shows it. It versions the **metadata structure**, never ghqc: a release that changes no metadata shape does not change the number. The shape above is version `1`; an absent field means the older pre-round shape, which is version `0`. A reader accepts exactly the structure versions it can interpret — today only `1` — and refuses every other value, **higher or lower**, with an error naming both, rather than parsing it best-effort: a misread QC archive is worse than one that says it cannot be read. Version `0` is refused for that reason and not as an oversight (see Action Required above), and the check runs in deserialization itself, so no read path can skip it
* `ghqc milestone status` now ends with an archive readiness line — `12 files · 9 approved & current · 2 approved but superseded · 1 unapproved (round 3 open)` — and `ghqc milestone archive` prints the same line before it writes. Both come from one categorization, so the pre-archive check cannot sort files differently from the archive it precedes. It is counted at each file's **latest** round, which is what an archive with no `--round` override would produce, and the output says so: a file approved in an earlier round and now under review again is counted **unapproved**, because unapproved content is what would be archived
* `ghqc milestone archive --round <issue#>=<n>` (repeatable) archives one issue's file at the round you name, 1-based with `1` being Initial QC; every issue you do not name is archived at its latest round. Interactively, the same choice is offered behind a single confirm and asked only for files that have more than one round. An earlier round's approval is now addressable at all — previously the only archivable commit was the newest approval, ever, so an archive matching an earlier QC round could not be reproduced once a later round approved
* `ghqc configuration edit [checklists|options|logo|record]` command to run a single step of the wizard against an existing configuration repository, located from `--config-dir`, the current directory, or the configured configuration directory; with no component named, they are offered as a menu
* `ghqc configuration init` command to interactively create a configuration repository — every `options.yaml` option, copying in a logo, the record template, and authoring markdown checklists in an editor, item by item, or from a bundled starter. Logo and template paths are picked with Tab-completing filesystem browsing. Re-running against an existing configuration repository turns it into an editor: current values become the prompt defaults, and existing checklists can be edited, renamed, or deleted. Files only; committing and pushing is left to the user
* Markdown checklists (`.md`, `.markdown`) are now loaded from the configuration repository, alongside the existing `.txt` and `.yaml`/`.yml` formats; the title comes from the filename and the file content is used as the checklist body verbatim
* `ghqc configuration update` command to fast-forward the local configuration repository to its remote; refuses and leaves the repository untouched when there are uncommitted changes, local commits not on the remote, or diverged history
* `ghqc configuration path` command to print just the configuration repository directory, for use in shells (`cd $(ghqc configuration path)`)
* POST /api/configuration/update endpoint backing the Web UI's configuration update button
* `allow_ui_config_update` option (env var `GHQC_ALLOW_CONFIG_UPDATE`, default `true`) to disable configuration repository updates from the Web UI in deployments where the configuration repository is managed centrally; the `ghqc configuration update` CLI command is deliberately unaffected

## Improvements
* `ghqc milestone archive` now archives each file at its **latest round** by default, rather than at its newest approval. A file that was approved and has since had a new round opened on it is therefore archived at *unapproved* content: there is a reason the round is open, and the archive shows current reality first. This is deliberately not gated, but it is never silent — every file's round, approval, commit and superseded state is printed before the archive is written, and `--round` (or the interactive round picker) targets the earlier round whose approval still stands
* A file with no approval is now archived at the newest commit someone **acted on** — the round's starting commit, a notification, or a review — instead of the newest commit on the branch. Drift nobody put up for review is no longer what an archive captures
* `ghqc milestone archive --include-unapproved` now means exactly "include files no round has ever closed on". A file approved in an earlier round is included either way; which round it is archived at is `--round`'s decision, not this flag's
* `ghqc milestone archive` names files whose **selected** round has no locatable commits, with the reason, and stops instead of quietly leaving them out of the archive. Only the selected round has to be locatable: a file whose earlier approval is a real commit is archived even when a later part of its history **cannot be established as current** — because it could not be read, or because it spans histories with no common ancestor — and that is recorded as `superseded` rather than treated as a reason to refuse the file. Acknowledging the callout (interactively, or with `--skip-unplaceable`) archives the rest **without** those files; a file whose round owns no commits has no commit an archive could honestly point at, so `--additional-file <path>:<commit>` is the way to archive one anyway
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
