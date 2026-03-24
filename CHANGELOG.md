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