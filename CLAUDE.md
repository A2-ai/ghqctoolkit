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
- **`src/cache.rs`** - Disk-based caching system for GitHub API responses with TTL support
- **`src/configuration.rs`** - Configuration file handling (YAML-based) with checklist management
- **`src/create.rs`** - Core business logic for milestone and issue creation
- **`src/issues.rs`** - `QCIssue` struct for GitHub issue creation with metadata
- **`src/utils.rs`** - Utility functions and environment provider abstractions
- **`src/git/`** - Git and GitHub integration layer
  - `action.rs` - Git repository operations (`GitAction` trait for clone/remote operations)  
  - `api.rs` - GitHub API trait definitions (`GitHubApi`, `RepoUser`)
  - `auth.rs` - GitHub authentication (token resolution from various sources)
  - `helpers.rs` - URL parsing and utility functions
  - `local.rs` - Local git operations using `gix` crate (`LocalGitInfo` trait)
  - `mod.rs` - Main `GitInfo` struct implementing all git traits
- **`src/cli/`** - CLI-specific functionality (gated by `cli` feature)
  - `context.rs` - CLI context and configuration management
  - `file_parser.rs` - File path parsing and validation
  - `interactive.rs` - Interactive prompts using `inquire` crate
  - `mod.rs` - CLI module exports and feature gating

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

### Caching System

The caching system (`src/cache.rs`) provides disk-based storage for GitHub API responses to improve performance and reduce API rate limiting:

- **`DiskCache`** - Main cache implementation using system cache directories (`~/.cache/ghqc/`)
- **`CacheEntry<T>`** - Generic cache entry with optional TTL (Time To Live) support
- **Hierarchical Storage** - Organized by owner/repo with subdirectories for different data types
- **Automatic Expiration** - Configurable TTL with automatic cleanup of expired entries
- **Environment Configuration** - TTL configurable via `GHQC_CACHE_TIMEOUT` (defaults to 1 hour)

Cache structure: `~/.cache/ghqc/{owner}/{repo}/{path}/{key}.json`

### Git Operations

#### Git Actions (`src/git/action.rs`)
- **`GitAction` trait** - Defines git repository operations (clone, remote URL extraction)
- **`GitActionImpl`** - Default implementation using `gix` library with authentication
- **Clone Operations** - Full repository cloning with authentication via GitHub tokens
- **Remote Management** - Extract remote URLs from existing repositories

#### Local Git Info (`src/git/local.rs`) 
- **`LocalGitInfo` trait** - Local git repository analysis (commit, branch, file history)
- **File Analysis** - Extract commit history and authors for specific files
- **Branch Detection** - Current branch name with fallback for detached HEAD
- **Author Extraction** - Unique authors from git history for file attribution

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

### CLI Module System

The CLI functionality (`src/cli/`) is feature-gated and provides interactive user interfaces:

#### CLI Components
- **`context.rs`** - CLI execution context with configuration and error handling
- **`file_parser.rs`** - File path validation and parsing for CLI arguments
- **`interactive.rs`** - Interactive prompts using `inquire` crate for user input
- **`mod.rs`** - Feature-gated module exports and CLI integration

#### Interactive Mode Features
- **File Selection** - Interactive file picker with validation
- **Milestone Management** - Create or select existing milestones
- **Checklist Selection** - Browse and select from available checklists
- **Assignee Management** - Multi-select assignees from repository collaborators

### Feature Gating

The `cli` feature gates all interactive dependencies:
- `clap`, `anyhow`, `inquire`, `clap-verbosity-flag`, `env_logger` - CLI-specific crates
- Interactive prompts only available with `--all-features`
- Library core functionality works without CLI feature
- Binary (`ghqc`) requires CLI feature and `--all-features` flag

### Utilities and Testing Support

#### Utils Module (`src/utils.rs`)
- **`EnvProvider` trait** - Mockable environment variable access for testing
- **`StdEnvProvider`** - Standard implementation using `std::env`
- **Testing Abstraction** - Enables mocking environment variables in unit tests

### Key Patterns

- **Trait-based Architecture**: `GitHubApi`, `LocalGitInfo`, `GitHelpers`, `GitAction`, `EnvProvider` traits enable testing and modularity
- **Async GitHub Operations**: All GitHub API calls are async with proper error handling
- **Concurrent User Fetching**: Repository assignees fetched in parallel for performance
- **Pagination Support**: GitHub API results properly paginated (100 per page)
- **Graceful Degradation**: User detail fetches fall back to login-only if name lookup fails
- **Comprehensive Mocking**: Uses `mockall` crate for trait-based mocking in tests
- **Disk Caching**: Performance optimization with automatic TTL management

### Important Implementation Notes

- `get_users()` fetches repository assignees (not contributors) via `/repos/{owner}/{repo}/assignees`
- Assignee validation happens before issue creation to fail fast
- Interactive mode requires all parameters (milestone, file, checklist) or none for full interactivity
- File authors extracted from git history, not GitHub API
- Issues are tagged with `["ghqc", branch_name]` labels automatically

### Dependencies and Configuration

#### Gix Features
The project uses `gix` 0.73 with specific features for git operations:
- `basic`, `extras` - Core git functionality
- `credentials` - Authentication support
- `worktree-mutation` - Working directory modifications
- `revision` - Required for revision walking and commit history analysis
- `blocking-network-client`, `blocking-http-transport-reqwest-rust-tls` - HTTPS cloning support

**Note**: The `revision` feature is essential for the `file_commits()` and `authors()` functions in `src/git/local.rs`. Without it, compilation will fail with type mismatches.