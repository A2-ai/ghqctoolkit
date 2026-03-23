use anyhow::{Result, anyhow, bail};
use inquire::Password;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::auth::{
    canonicalize_base_url, delete_token, extract_host_from_base_url, save_token, token_page_url,
};

pub fn gh_auth_login(
    directory: &Path,
    host: Option<&str>,
    token: Option<&str>,
    no_store: bool,
) -> Result<()> {
    let base_url = resolve_host_from_repo_or_flag(host, directory)?;
    let host_name = extract_host_from_base_url(&base_url)?;

    if let Some(token) = token {
        if no_store {
            warn_no_store_ignored();
        }
        let validated = validate_token_or_error(token)?;
        save_token(&base_url, &validated)?;
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
                save_token(&base_url, &token)?;
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
    save_token(&base_url, &token)?;
    println!("Stored token for {}", host_name);
    Ok(())
}

pub fn gh_auth_logout(directory: &Path, host: Option<&str>) -> Result<()> {
    let base_url = resolve_host_from_repo_or_flag(host, directory)?;
    let host_name = extract_host_from_base_url(&base_url)?;

    if delete_token(&base_url)? {
        println!("Removed stored token for {}", host_name);
    } else {
        println!("No ghqc-stored token found for {}", host_name);
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

    let env = crate::utils::StdEnvProvider;
    let git_info = crate::GitInfo::from_path(directory, &env).map_err(|_| {
        anyhow!("Could not determine host from repository. Re-run with --host <host>.")
    })?;
    Ok(git_info.base_url.clone())
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

#[cfg(test)]
mod tests {
    #[test]
    fn gh_token_output_is_valid() {
        assert!(
            crate::validate_github_token("ghp_1234567890123456789012345678901234567890").is_some()
        );
        assert!(crate::validate_github_token("bad token").is_none());
    }
}
