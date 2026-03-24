use anyhow::{Result, anyhow, bail};
use inquire::Password;
use owo_colors::OwoColorize;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::AuthSources;
use crate::auth::{AuthStore, canonicalize_base_url, extract_host_from_base_url, token_page_url};
use crate::utils::StdEnvProvider;
use super::section_header;

pub fn gh_auth_login(
    directory: &Path,
    host: Option<&str>,
    token: Option<&str>,
    no_store: bool,
    store: &AuthStore,
) -> Result<()> {
    let base_url = resolve_host_from_repo_or_flag(host, directory)?;
    let host_name = extract_host_from_base_url(&base_url)?;

    if let Some(token) = token {
        if no_store {
            warn_no_store_ignored();
        }
        let validated = validate_token_or_error(token)?;
        store.save_token(&host_name, &validated)?;
        println!("Stored token for {}", host_name);
        return Ok(());
    }

    if gh_available() {
        run_gh_login(&host_name)?;
        if no_store {
            println!(
                "gh login completed for {}. Skipped ghqc storage due to --no-store.",
                host_name
            );
            return Ok(());
        }

        match read_gh_token(&host_name)? {
            Some(token) => {
                store.save_token(&host_name, &token)?;
                println!(
                    "gh login completed and token imported into ghqc for {}",
                    host_name
                );
            }
            None => {
                eprintln!(
                    "Warning: gh login completed for {}, but ghqc could not read a token to import.",
                    host_name
                );
            }
        }
        return Ok(());
    }

    if no_store {
        warn_no_store_ignored();
    }

    eprintln!(
        "gh was not found. ghqc will open the personal access token page for {} and then prompt for a token.",
        host_name
    );
    let token_url = token_page_url(&base_url)?;
    wait_for_enter()?;
    let _ = open::that(&token_url);
    eprintln!("Token URL: {}", token_url);

    let token = Password::new("GitHub token:")
        .without_confirmation()
        .with_help_message("Paste a personal access token for this host. Input is hidden.")
        .prompt()
        .map_err(|e| anyhow!("Prompt cancelled: {e}"))?;

    let token = require_nonempty_token(token)?;
    store.save_token(&host_name, &token)?;
    println!("Stored token for {}", host_name);
    Ok(())
}

pub fn gh_auth_logout(directory: &Path, host: Option<&str>, store: &AuthStore) -> Result<()> {
    let base_url = resolve_host_from_repo_or_flag(host, directory)?;
    let host_name = extract_host_from_base_url(&base_url)?;

    if store.delete_token(&host_name)? {
        println!("Removed stored token for {}", host_name);
    } else {
        println!("No ghqc-stored token found for {}", host_name);
    }
    Ok(())
}

pub fn gh_auth_status(directory: &Path, host: Option<&str>, store: &AuthStore) -> Result<()> {
    let selected_base_url = try_resolve_host_from_repo_or_flag(host, directory);
    let selected_host = selected_base_url
        .as_deref()
        .and_then(|base_url| extract_host_from_base_url(base_url).ok());

    println!("{}", section_header("Auth Store"));
    println!("store directory: {}", store.root.display());

    let stored = store.display_with_selected(selected_host.as_deref());
    if stored == "none" {
        println!("stored tokens: none");
    } else {
        println!("stored tokens:");
        for line in stored.lines() {
            println!("{line}");
        }
        println!();
    }

    println!("{}", section_header("Auth Sources"));
    if let Some(base_url) = &selected_base_url {
        let host = selected_host.as_ref().unwrap_or(base_url);
        println!("repository host: {host}");
        let auth_sources = AuthSources::new(base_url, &StdEnvProvider, Some(store));
        print_host_auth(&auth_sources);
    } else {
        println!("  no host selected — run from a git repo or pass --host");
    }

    Ok(())
}

fn gh_available() -> bool {
    Command::new("gh")
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn run_gh_login(host_name: &str) -> Result<()> {
    let status = Command::new("gh")
        .args(["auth", "login", "--hostname", host_name])
        .env("GH_HOST", host_name)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| anyhow!("Failed to start gh auth login: {e}"))?;

    if !status.success() {
        bail!("gh auth login failed for {}", host_name);
    }
    Ok(())
}

fn read_gh_token(host_name: &str) -> Result<Option<String>> {
    let output = Command::new("gh")
        .args(["auth", "token", "--hostname", host_name])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| anyhow!("Failed to read token from gh: {e}"))?;

    if !output.status.success() {
        return Ok(None);
    }

    let token = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if token.is_empty() {
        return Ok(None);
    }
    Ok(crate::validate_github_token(&token))
}

fn resolve_host_from_repo_or_flag(host: Option<&str>, directory: &Path) -> Result<String> {
    if let Some(host) = host {
        return canonicalize_base_url(host).map_err(Into::into);
    }
    resolve_host_from_repo(directory)
}

fn try_resolve_host_from_repo_or_flag(host: Option<&str>, directory: &Path) -> Option<String> {
    resolve_host_from_repo_or_flag(host, directory).ok()
}

fn validate_token_or_error(token: &str) -> Result<String> {
    crate::validate_github_token(token)
        .ok_or_else(|| anyhow!("Provided token is not a valid GitHub token"))
}

fn warn_no_store_ignored() {
    eprintln!(
        "Warning: --no-store only applies to the gh shell-out login flow and is ignored here."
    );
}

fn wait_for_enter() -> Result<()> {
    use std::io;
    eprintln!("Press Enter to continue.");
    let mut input = String::new();
    io::stdin()
        .read_line(&mut input)
        .map_err(|e| anyhow!("Failed to read confirmation input: {e}"))?;
    Ok(())
}

fn require_nonempty_token(token: String) -> Result<String> {
    let token = token.trim().to_string();
    if token.is_empty() {
        bail!("Token input was empty");
    }
    validate_token_or_error(&token)
}

fn print_host_auth(auth_sources: &AuthSources) {
    let active = auth_sources
        .sorted()
        .into_iter()
        .next()
        .map(|(k, _)| k.to_string());
    println!();
    println!("available auth sources");
    for (kind, token) in auth_sources.all_by_priority() {
        let is_active = active.as_deref() == Some(&kind.to_string());
        match token {
            Some(t) => println!(
                "{}{} {} ({})",
                if is_active {
                    "▶ ".green().to_string()
                } else {
                    "  ".to_string()
                },
                "✓".green(),
                if is_active {
                    format!("{kind:<26}").bold().to_string()
                } else {
                    format!("{kind:<26}")
                },
                crate::auth::preview_token(t)
            ),
            None => println!("  {} {kind}", "✗".red()),
        }
    }
}

fn resolve_host_from_repo(directory: &Path) -> Result<String> {
    let repository = gix::open(directory).map_err(|_| {
        anyhow!("Could not determine host from repository. Re-run with --host <host>.")
    })?;
    let remote = repository
        .find_default_remote(gix::remote::Direction::Fetch)
        .ok_or_else(|| {
            anyhow!("Could not determine host from repository. Re-run with --host <host>.")
        })?
        .map_err(|_| {
            anyhow!("Could not determine host from repository. Re-run with --host <host>.")
        })?;
    let remote_url = remote
        .url(gix::remote::Direction::Fetch)
        .ok_or_else(|| {
            anyhow!("Could not determine host from repository. Re-run with --host <host>.")
        })?
        .to_string();

    parse_remote_base_url(&remote_url).ok_or_else(|| {
        anyhow!("Could not determine host from repository. Re-run with --host <host>.")
    })
}

fn parse_remote_base_url(url: &str) -> Option<String> {
    let url = url.strip_suffix(".git").unwrap_or(url);

    if let Some(captures) = url.strip_prefix("https://") {
        let parts: Vec<&str> = captures.split('/').collect();
        if parts.len() >= 3 {
            return Some(format!("https://{}", parts[0]));
        }
    }

    if let Some(captures) = url.strip_prefix("git@") {
        let host_and_path: Vec<&str> = captures.split(':').collect();
        if host_and_path.len() == 2 {
            let path_parts: Vec<&str> = host_and_path[1].split('/').collect();
            if path_parts.len() >= 2 {
                return Some(format!("https://{}", host_and_path[0]));
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::{parse_remote_base_url, try_resolve_host_from_repo_or_flag};
    use tempfile::TempDir;

    #[test]
    fn gh_token_output_is_valid() {
        assert!(
            crate::validate_github_token("ghp_1234567890123456789012345678901234567890").is_some()
        );
        assert!(crate::validate_github_token("bad token").is_none());
    }

    #[test]
    fn try_resolve_host_uses_explicit_host() {
        let temp = TempDir::new().unwrap();
        let resolved = try_resolve_host_from_repo_or_flag(Some("github.com"), temp.path()).unwrap();
        assert_eq!(resolved, "https://github.com");
    }

    #[test]
    fn try_resolve_host_returns_none_when_repo_host_is_unavailable() {
        let temp = TempDir::new().unwrap();
        assert_eq!(try_resolve_host_from_repo_or_flag(None, temp.path()), None);
    }

    #[test]
    fn parse_remote_base_url_supports_https_and_ssh() {
        assert_eq!(
            parse_remote_base_url("https://github.com/owner/repo.git"),
            Some("https://github.com".to_string())
        );
        assert_eq!(
            parse_remote_base_url("git@ghe.example.com:owner/repo.git"),
            Some("https://ghe.example.com".to_string())
        );
    }
}
