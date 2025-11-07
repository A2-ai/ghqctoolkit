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

#### Issue Creation
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
```

#### Issue Comments and Updates
```bash
# Interactive comment mode (prompts for milestone, issue, commits)
cargo run --all-features -- issue comment

# Comment with specific commits and note
cargo run --all-features -- issue comment \
  --milestone "v1.0" \
  --file "src/main.rs" \
  --current-commit abc123 \
  --previous-commit def456 \
  --note "Fixed validation logic"

# Approve and close an issue (interactive)
cargo run --all-features -- issue approve

# Approve with specific commit
cargo run --all-features -- issue approve \
  --milestone "v1.0" \
  --file "src/main.rs" \
  --approved-commit abc123 \
  --note "Code review passed"

# Unapprove and reopen an issue (interactive)
cargo run --all-features -- issue unapprove

# Unapprove with reason
cargo run --all-features -- issue unapprove \
  --milestone "v1.0" \
  --file "src/main.rs" \
  --reason "Security vulnerability found"

# Review working directory changes against a commit (interactive)
cargo run --all-features -- issue review

# Review with specific commit (defaults to HEAD)
cargo run --all-features -- issue review \
  --milestone "v1.0" \
  --file "src/main.rs" \
  --commit abc123 \
  --note "Testing local changes"

# Review without diff output
cargo run --all-features -- issue review \
  --milestone "v1.0" \
  --file "src/main.rs" \
  --no-diff
```

#### Status and Information
```bash
# Check specific issue status
cargo run --all-features -- issue status \
  --milestone "v1.0" \
  --file "src/main.rs"

# Interactive issue status (prompts for milestone and issue)
cargo run --all-features -- issue status

# Check milestone status
cargo run --all-features -- milestone status "v1.0"

# Check all milestones
cargo run --all-features -- milestone status --all-milestones

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
- **`src/create.rs`** - Core business logic for milestone and issue creation (`QCIssue`)
- **`src/comment.rs`** - Comment functionality for commit-to-commit diffs (`QCComment`)
- **`src/approve.rs`** - Approval and unapproval functionality (`QCApprove`, `QCUnapprove`)
- **`src/review.rs`** - Review functionality for commit-to-local diffs (`QCReview`)
- **`src/comment_system.rs`** - Unified comment system with `CommentBody` trait
- **`src/diff_utils.rs`** - Shared diff generation utilities for Excel and text files
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

### Unified Comment System Architecture

The comment system uses a unified architecture to eliminate code duplication and provide consistent behavior across all comment types:

#### CommentBody Trait (`src/comment_system.rs`)
- **`CommentBody` trait** - Unified interface for all comment types
- **`generate_body()`** - Generates markdown comment body requiring both `GitHelpers` + `GitFileOps`
- **`issue()`** - Returns reference to associated GitHub issue
- **Consistent formatting** - All comments follow the same structure and metadata patterns

#### Comment Types
- **`QCComment`** - Commit-to-commit diff comments for progress updates
- **`QCApprove`** - Approval comments that close issues when posted
- **`QCUnapprove`** - Unapproval comments that reopen closed issues
- **`QCReview`** - Commit-to-local diff comments for reviewing working directory changes

#### Shared Components
- **`diff_utils.rs`** - Unified diff generation for Excel and text files
- **Enhanced GitHelpers** - URL generation for commits and file links
- **Robust error handling** - Detailed logging for file access and diff generation issues

#### GitHubWriter Trait Separation
- **`post_comment<T: CommentBody>()`** - Unified comment posting for any comment type
- **`close_issue()`** - Separate issue state management for approvals
- **`open_issue()`** - Separate issue state management for unapprovals
- **Clean architecture** - Comment posting decoupled from issue state changes

### Key Data Flow

1. **Initialization**: `GitInfo::from_path()` analyzes local git repo and creates authenticated GitHub client
2. **User Input**: Either CLI args or interactive prompts gather milestone, file, checklist, and assignees
3. **Validation**: Assignees validated against repository collaborators via GitHub API
4. **Issue/Comment Creation**: Comment types implement `CommentBody` trait for consistent formatting
5. **GitHub Integration**: Unified comment posting with separate issue state management

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
- **Issue Creation** - Complete issue creation workflow with file selection, milestone management, and assignee selection
- **Comment Management** - Select issues within milestones and choose commits for comparison with status annotations
- **Approval Workflow** - Select open issues and commits to approve with robust fallback logic for commit selection
- **Unapproval Process** - Select closed issues to reopen with required reason validation
- **Review Mode** - Compare working directory changes against commits with commit selection and diff options
- **Commit Selection** - Interactive commit picker with status indicators (🌱 Initial, 💬 Comments, ✅ Approved, 📍 Latest, 📝 File changed)
- **Robust Defaults** - Smart fallback chain for commit selection: HEAD → latest_commit → position 0

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

- **Trait-based Architecture**: `GitHubApi`, `LocalGitInfo`, `GitHelpers`, `GitAction`, `EnvProvider`, `CommentBody` traits enable testing and modularity
- **Unified Comment System**: `CommentBody` trait eliminates code duplication across comment types with consistent formatting
- **Separation of Concerns**: Comment generation separate from issue state management (close/open operations)
- **Shared Utilities**: `diff_utils.rs` provides unified diff generation for Excel and text files
- **Robust Fallback Logic**: Smart defaults for commit selection with graceful error handling
- **Enhanced Error Logging**: Detailed logging for file access and diff generation issues
- **Async GitHub Operations**: All GitHub API calls are async with proper error handling
- **Concurrent User Fetching**: Repository assignees fetched in parallel for performance
- **Pagination Support**: GitHub API results properly paginated (100 per page)
- **Graceful Degradation**: User detail fetches fall back to login-only if name lookup fails
- **Comprehensive Mocking**: Uses `mockall` crate for trait-based mocking in tests with both `GitHelpers` and `GitFileOps`
- **Disk Caching**: Performance optimization with automatic TTL management

### Important Implementation Notes

- `get_users()` fetches repository assignees (not contributors) via `/repos/{owner}/{repo}/assignees`
- Assignee validation happens before issue creation to fail fast
- Interactive mode requires all parameters (milestone, file, checklist) or none for full interactivity
- File authors extracted from git history, not GitHub API
- Issues are tagged with `["ghqc", branch_name]` labels automatically

#### Comment System Specifics
- **QCComment**: Compares two commits for progress tracking
- **QCReview**: Compares commit against working directory for pre-commit review
- **QCApprove**: Posts approval and closes issue atomically
- **QCUnapprove**: Posts unapproval and reopens issue atomically
- **Commit Selection**: Robust fallback chain prevents command failures from missing defaults
- **Diff Generation**: Supports both Excel (.xlsx) and text files with unified error handling
- **Interactive Prompts**: Issue selection within milestones (not file creation like issue create)
- **Status Indicators**: Commit picker shows visual status (Initial, Comments, Approved, Latest, File changed)

### Dependencies and Configuration

#### Gix Features
The project uses `gix` 0.73 with specific features for git operations:
- `basic`, `extras` - Core git functionality
- `credentials` - Authentication support
- `worktree-mutation` - Working directory modifications
- `revision` - Required for revision walking and commit history analysis
- `blocking-network-client`, `blocking-http-transport-reqwest-rust-tls` - HTTPS cloning support

**Note**: The `revision` feature is essential for the `file_commits()` and `authors()` functions in `src/git/local.rs`. Without it, compilation will fail with type mismatches.