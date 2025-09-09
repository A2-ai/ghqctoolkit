# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Development Commands

### Building and Testing
```bash
# Build with all features (required for CLI)
cargo build --all-features

# Run all tests
cargo test --all-features

# Run specific test module
cargo test --all-features create::tests::test_create_issue_matrix

# Run the CLI binary (requires --all-features)
cargo run --all-features -- issue create --help
```

### Common CLI Usage Examples
```bash
# Interactive mode (prompts for all inputs)
cargo run --all-features -- issue create

# Non-interactive mode (all parameters required)
cargo run --all-features -- issue create \
  --milestone "v1.0" \
  --file "src/main.rs" \
  --checklist-name "Simple Tasks" \
  --assignees user1 user2

# With custom configuration
cargo run --all-features -- issue create \
  --config-dir ./custom-config \
  --milestone "Sprint 1" \
  --file "src/lib.rs" \
  --checklist-name "Code Review"

# Check help for all issue commands
cargo run --all-features -- issue --help
```

## Architecture Overview

### Core Components

**ghqctoolkit** is a CLI tool for creating GitHub issues with quality control checklists. It integrates with both local git repositories and GitHub APIs to automate issue creation with file-specific context.

### Module Structure

- **`src/main.rs`** - CLI entry point using clap for argument parsing
- **`src/lib.rs`** - Library exports with conditional CLI feature gating
- **`src/git/`** - Git and GitHub integration layer
  - `api.rs` - GitHub API trait definitions (`GitHubApi`, `RepoUser`)
  - `auth.rs` - GitHub authentication (token resolution from various sources)
  - `helpers.rs` - URL parsing and utility functions
  - `local.rs` - Local git operations using `gix` crate
  - `mod.rs` - Main `GitInfo` struct implementing all git traits
- **`src/issues.rs`** - `QCIssue` struct for GitHub issue creation
- **`src/create.rs`** - Core business logic for milestone and issue creation
- **`src/configuration.rs`** - Configuration file handling (YAML-based)
- **`src/cli/interactive.rs`** - Interactive prompts using `inquire` crate

### Key Data Flow

1. **Initialization**: `GitInfo::from_path()` analyzes local git repo and creates authenticated GitHub client
2. **User Input**: Either CLI args or interactive prompts gather milestone, file, checklist, and assignees
3. **Validation**: Assignees validated against repository collaborators via GitHub API
4. **Issue Creation**: `QCIssue` combines file metadata (authors, commit) with checklist content
5. **GitHub Integration**: Issue posted with milestone, assignees, and labels

### Configuration System

Configuration is YAML-based with two components:
- **`options.yaml`** - Global settings (checklist notes, display names, paths)
- **`checklists/`** - Directory containing checklist templates (`.yaml`, `.yml`, `.txt`, `.md`)

Checklist files can be:
- Simple text files with markdown-style checkboxes
- YAML files with structured checklist data
- Supports complex names with spaces and special characters

### Testing Architecture

- **Matrix Testing**: `test_create_issue_matrix()` covers milestone scenarios (existing, new, unknown)
- **Mock Integration**: Uses `mockall` for GitHub API mocking
- **Fixture System**: JSON fixtures in `src/tests/github_api/` for realistic API responses
- **Validation Tests**: Separate tests for assignee validation logic

### Authentication Flow

GitHub authentication follows priority order:
1. Environment variable `GITHUB_TOKEN`
2. GitHub CLI token (`gh auth token`)
3. `.netrc` file credentials
4. Falls back to unauthenticated (limited functionality)

### Feature Gating

The `cli` feature gates all interactive dependencies:
- `clap`, `anyhow`, `inquire` - CLI-specific crates
- Interactive prompts only available with `--all-features`
- Library core functionality works without CLI feature

### Key Patterns

- **Trait-based Architecture**: `GitHubApi`, `LocalGitInfo`, `GitHelpers` traits enable testing and modularity
- **Async GitHub Operations**: All GitHub API calls are async with proper error handling
- **Concurrent User Fetching**: Repository assignees fetched in parallel for performance
- **Pagination Support**: GitHub API results properly paginated (100 per page)
- **Graceful Degradation**: User detail fetches fall back to login-only if name lookup fails

### Important Implementation Notes

- `get_users()` fetches repository assignees (not contributors) via `/repos/{owner}/{repo}/assignees`
- Assignee validation happens before issue creation to fail fast
- Interactive mode requires all parameters (milestone, file, checklist) or none for full interactivity
- File authors extracted from git history, not GitHub API
- Issues are tagged with `["ghqc", branch_name]` labels automatically