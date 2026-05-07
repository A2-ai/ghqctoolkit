use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow, bail};
use clap::{Subcommand, ValueEnum};

use crate::cache::cache_root;
use crate::git::GitInfo;
use crate::git::GitRepository;
use crate::utils::StdEnvProvider;

#[derive(Subcommand)]
pub enum CacheCommands {
    /// Remove cached data from disk
    #[command(alias = "rm")]
    Remove {
        /// Which cache element to clear. Omit to clear all caches for the current repo
        /// (or, with --global, the entire ghqc cache directory).
        #[arg(value_enum)]
        element: Option<CacheElement>,

        /// Clear across every owner/repo. With <feature>: removes that feature for every
        /// repo. Without <feature>: wipes the entire ghqc cache directory.
        #[arg(long)]
        global: bool,
    },
    /// Print the cache directory for the current repo (or --global for the root)
    #[command(alias = "directory")]
    Dir {
        /// Print the ghqc cache root instead of the per-repo directory.
        #[arg(long)]
        global: bool,
    },
    /// Show cache locations, sizes, and TTL settings
    Status,
}

#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum CacheElement {
    /// Per-commit file-change records (drives the "commits that changed file X" list).
    Commits,
    /// Cached issue comments and events.
    Issues,
    /// Repo assignees and user details.
    Users,
    /// Repo labels.
    Labels,
}

impl CacheElement {
    fn dir_name(self) -> &'static str {
        match self {
            CacheElement::Commits => "commits",
            CacheElement::Issues => "issues",
            CacheElement::Users => "users",
            CacheElement::Labels => "labels",
        }
    }
}

pub fn handle_cache(cmd: CacheCommands, directory: &Path) -> Result<()> {
    match cmd {
        CacheCommands::Remove { element, global } => clear(element, global, directory),
        CacheCommands::Dir { global } => dir(global, directory),
        CacheCommands::Status => status(directory),
    }
}

fn status(directory: &Path) -> Result<()> {
    let root = cache_root().map_err(|e| anyhow!("failed to resolve cache root: {e}"))?;

    println!("{}", super::section_header("Cache"));
    println!("root:     {}", root.display());
    if root.exists() {
        let (size, files) = dir_stats(&root)?;
        println!(
            "size:     {} ({} file{})",
            format_bytes(size),
            files,
            if files == 1 { "" } else { "s" }
        );
    } else {
        println!("size:     (cache root does not exist yet)");
    }
    println!("ttl:      {}", ttl_description());

    println!("{}", super::section_header("Repository"));
    match resolve_repo(directory) {
        Ok((owner, repo)) => {
            let repo_dir = root.join(&owner).join(&repo);
            println!("repo:     {}/{}", owner, repo);
            println!("path:     {}", repo_dir.display());
            if !repo_dir.exists() {
                println!("(no cache entries for this repo yet)");
            } else {
                println!();
                println!("  {:<10} {:>10} {:>8}", "element", "size", "files");
                println!("  {:<10} {:>10} {:>8}", "-------", "----", "-----");
                for elem in [
                    CacheElement::Commits,
                    CacheElement::Issues,
                    CacheElement::Users,
                    CacheElement::Labels,
                ] {
                    let p = repo_dir.join(elem.dir_name());
                    let (size, files) = if p.exists() { dir_stats(&p)? } else { (0, 0) };
                    let size_str = if files == 0 {
                        "—".to_string()
                    } else {
                        format_bytes(size)
                    };
                    let files_str = if files == 0 {
                        "—".to_string()
                    } else {
                        files.to_string()
                    };
                    println!(
                        "  {:<10} {:>10} {:>8}",
                        elem.dir_name(),
                        size_str,
                        files_str
                    );
                }
            }
        }
        Err(_) => {
            println!("(not in a git repository — run from inside a repo for per-repo stats)");
        }
    }

    Ok(())
}

fn dir_stats(path: &Path) -> Result<(u64, u64)> {
    let mut size = 0u64;
    let mut files = 0u64;
    let mut stack = vec![path.to_path_buf()];
    while let Some(p) = stack.pop() {
        let meta = fs::symlink_metadata(&p)?;
        if meta.is_dir() {
            for entry in fs::read_dir(&p)? {
                stack.push(entry?.path());
            }
        } else if meta.is_file() {
            size += meta.len();
            files += 1;
        }
    }
    Ok((size, files))
}

fn format_bytes(n: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    if n >= GB {
        format!("{:.2} GB", n as f64 / GB as f64)
    } else if n >= MB {
        format!("{:.2} MB", n as f64 / MB as f64)
    } else if n >= KB {
        format!("{:.1} KB", n as f64 / KB as f64)
    } else {
        format!("{} B", n)
    }
}

fn ttl_description() -> String {
    match std::env::var("GHQC_CACHE_TIMEOUT") {
        Ok(v) => format!("{}s (from GHQC_CACHE_TIMEOUT)", v),
        Err(_) => "3600s (default; override with GHQC_CACHE_TIMEOUT)".to_string(),
    }
}

fn dir(global: bool, directory: &Path) -> Result<()> {
    let root = cache_root().map_err(|e| anyhow!("failed to resolve cache root: {e}"))?;
    let path = if global {
        root
    } else {
        let (owner, repo) = resolve_repo(directory)?;
        root.join(owner).join(repo)
    };
    println!("{}", path.display());
    Ok(())
}

fn clear(element: Option<CacheElement>, global: bool, directory: &Path) -> Result<()> {
    let root = cache_root().map_err(|e| anyhow!("failed to resolve cache root: {e}"))?;

    let removed = match (global, element) {
        (true, None) => remove_dir(&root)?.into_iter().collect::<Vec<_>>(),
        (true, Some(f)) => clear_feature_global(&root, f)?,
        (false, None) => {
            let (owner, repo) = resolve_repo(directory)?;
            remove_dir(&root.join(&owner).join(&repo))?
                .into_iter()
                .collect()
        }
        (false, Some(f)) => {
            let (owner, repo) = resolve_repo(directory)?;
            remove_dir(&root.join(&owner).join(&repo).join(f.dir_name()))?
                .into_iter()
                .collect()
        }
    };

    if removed.is_empty() {
        println!("no cache entries found");
    } else {
        for path in &removed {
            println!("removed {}", path.display());
        }
        println!(
            "cleared {} cache director{}",
            removed.len(),
            if removed.len() == 1 { "y" } else { "ies" }
        );
    }
    Ok(())
}

fn resolve_repo(directory: &Path) -> Result<(String, String)> {
    let env = StdEnvProvider;
    let git_info = GitInfo::from_path(directory, &env, None).map_err(|e| {
        anyhow!(
            "not in a git repository (or repo info unavailable): {e}. \
             Use --global to operate on the entire ghqc cache."
        )
    })?;
    Ok((git_info.owner().to_string(), git_info.repo().to_string()))
}

fn clear_feature_global(root: &Path, element: CacheElement) -> Result<Vec<PathBuf>> {
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut removed = Vec::new();
    for owner_entry in fs::read_dir(root)? {
        let owner_path = owner_entry?.path();
        if !owner_path.is_dir() {
            continue;
        }
        for repo_entry in fs::read_dir(&owner_path)? {
            let repo_path = repo_entry?.path();
            if !repo_path.is_dir() {
                continue;
            }
            let target = repo_path.join(element.dir_name());
            if let Some(p) = remove_dir(&target)? {
                removed.push(p);
            }
        }
    }
    Ok(removed)
}

/// Remove `path` if it exists. Returns the path if something was removed, None otherwise.
fn remove_dir(path: &Path) -> Result<Option<PathBuf>> {
    match fs::remove_dir_all(path) {
        Ok(()) => Ok(Some(path.to_path_buf())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => bail!("failed to remove {}: {e}", path.display()),
    }
}
