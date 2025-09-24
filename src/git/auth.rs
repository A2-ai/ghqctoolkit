use crate::utils::EnvProvider;
use octocrab::Octocrab;
use std::path::PathBuf;
use std::process::Command;

#[derive(thiserror::Error, Debug)]
pub enum AuthError {
    #[error("Failed to build octocrab client: {0}")]
    ClientBuild(#[from] octocrab::Error),
    #[error(
        "No authentication found. Try: GITHUB_TOKEN env var, 'gh auth login', or git credential manager"
    )]
    NoAuth,
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON parsing error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("YAML parsing error: {0}")]
    Yaml(#[from] serde_yaml::Error),
}

pub fn create_authenticated_client(
    base_url: &str,
    token: Option<String>,
) -> Result<Octocrab, AuthError> {
    match token {
        Some(token) => build_client_with_token(base_url, token),
        None => {
            log::warn!(
                "No authentication found. API access will be limited to public repositories"
            );
            // Fall back to unauthenticated client
            build_unauthenticated_client(base_url)
        }
    }
}

pub fn get_token(base_url: &str, env: &impl EnvProvider) -> Option<String> {
    // Try authentication sources in priority order (similar to gitcreds R package)

    // 1. GITHUB_TOKEN environment variable (highest priority)
    log::debug!("Trying from GITHUB_TOKEN");
    if let Ok(token) = env.var("GITHUB_TOKEN") {
        log::debug!("Using GITHUB_TOKEN environment variable");
        return Some(token);
    }

    // 2. gh CLI authentication
    log::debug!("Trying from gh cli authentication");
    if let Some(token) = get_gh_token_with_env(base_url, env) {
        log::debug!("Using gh CLI stored credentials");
        return Some(token);
    }

    // 3. Git credential manager (git credential fill)
    log::debug!("Trying from git credential manager");
    if let Some(token) = get_git_credential_token(base_url) {
        log::debug!("Using git credential manager");
        return Some(token);
    }

    // 4. .netrc file
    log::debug!("Trying from .netrc file");
    if let Some(token) = get_netrc_token_with_env(base_url, env) {
        log::debug!("Using .netrc file credentials");
        return Some(token);
    }

    log::warn!("No authentication found. API access will be limited to public repositories");

    None
}

fn build_client_with_token(base_url: &str, token: String) -> Result<Octocrab, AuthError> {
    // Assume we're already in a proper tokio runtime context (called from API functions)
    log::debug!("Creating Octocrab client (assuming proper runtime context)");
    if base_url == "https://github.com" {
        Octocrab::builder().personal_token(token).build()
    } else {
        Octocrab::builder()
            .base_uri(format!("{}/api/v3", base_url))
            .and_then(|b| b.personal_token(token).build())
    }
    .map_err(AuthError::ClientBuild)
}

fn build_unauthenticated_client(base_url: &str) -> Result<Octocrab, AuthError> {
    // Assume we're already in a proper tokio runtime context (called from API functions)
    log::debug!("Creating unauthenticated Octocrab client (assuming proper runtime context)");
    if base_url == "https://github.com" {
        Octocrab::builder().build()
    } else {
        Octocrab::builder()
            .base_uri(format!("{}/api/v3", base_url))
            .and_then(|b| b.build())
    }
    .map_err(AuthError::ClientBuild)
}

fn get_gh_token_with_env(base_url: &str, env: &impl EnvProvider) -> Option<String> {
    let config_dir = get_gh_config_dir_with_env(env)?;
    let hosts_file = config_dir.join("hosts.yml");

    if !hosts_file.exists() {
        return None;
    }

    let content = std::fs::read_to_string(&hosts_file).ok()?;
    let hosts: serde_yaml::Value = serde_yaml::from_str(&content).ok()?;

    let host = extract_host_from_url(base_url);

    if let Some(host_config) = hosts.get(host) {
        if let Some(oauth_token) = host_config.get("oauth_token") {
            if let Some(token_str) = oauth_token.as_str() {
                return Some(token_str.to_string());
            }
        }
    }

    None
}

fn get_git_credential_token(base_url: &str) -> Option<String> {
    // Skip git credential fill in R/extendr context to avoid subprocess crashes
    // Check for common R environment indicators
    if std::env::var("R_HOME").is_ok()
        || std::env::var("R_SESSION_TMPDIR").is_ok()
        || std::env::var("RSTUDIO").is_ok()
    {
        log::debug!("Skipping git credential manager in R environment to avoid subprocess issues");
        return None;
    }

    let host = extract_host_from_url(base_url);

    // Use git credential fill to get stored credentials
    let mut cmd = Command::new("git");
    cmd.args(&["credential", "fill"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    let input = format!("protocol=https\nhost={}\n\n", host);

    // Add extra error handling to prevent crashes
    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(e) => {
            log::debug!("Failed to spawn git credential process: {}", e);
            return None;
        }
    };

    if let Some(stdin) = child.stdin.as_mut() {
        use std::io::Write;
        if stdin.write_all(input.as_bytes()).is_err() {
            log::debug!("Failed to write to git credential stdin");
            return None;
        }
    }

    let output = match child.wait_with_output() {
        Ok(output) => output,
        Err(e) => {
            log::debug!("Failed to wait for git credential output: {}", e);
            return None;
        }
    };

    if !output.status.success() {
        log::debug!(
            "Git credential command failed with status: {}",
            output.status
        );
        return None;
    }

    let output_str = String::from_utf8_lossy(&output.stdout);

    // Parse the credential output for password (token)
    for line in output_str.lines() {
        if let Some(password) = line.strip_prefix("password=") {
            // GitHub tokens should start with specific prefixes
            if password.starts_with("ghp_")
                || password.starts_with("github_pat_")
                || password.starts_with("gho_")
                || password.starts_with("ghu_")
            {
                return Some(password.to_string());
            }
        }
    }

    None
}

fn get_netrc_token_with_env(base_url: &str, env: &impl EnvProvider) -> Option<String> {
    let host = extract_host_from_url(base_url);
    let netrc_path = get_netrc_path_with_env(env)?;

    if !netrc_path.exists() {
        return None;
    }

    let content = std::fs::read_to_string(&netrc_path).ok()?;

    // Parse .netrc format: machine <host> login <user> password <token>
    let lines: Vec<&str> = content.split_whitespace().collect();
    let mut i = 0;

    while i < lines.len() {
        if lines[i] == "machine" && i + 1 < lines.len() {
            let machine_host = lines[i + 1];
            if machine_host == host {
                // Found matching host, look for password
                let mut j = i + 2;
                while j < lines.len() && lines[j] != "machine" {
                    if lines[j] == "password" && j + 1 < lines.len() {
                        let password = lines[j + 1];
                        // Check if it looks like a GitHub token
                        if password.starts_with("ghp_")
                            || password.starts_with("github_pat_")
                            || password.starts_with("gho_")
                            || password.starts_with("ghu_")
                        {
                            return Some(password.to_string());
                        }
                    }
                    j += 1;
                }
            }
        }
        i += 1;
    }

    None
}

fn extract_host_from_url(base_url: &str) -> &str {
    if base_url == "https://github.com" {
        "github.com"
    } else {
        base_url.strip_prefix("https://").unwrap_or(base_url)
    }
}

fn get_gh_config_dir_with_env(env: &impl EnvProvider) -> Option<PathBuf> {
    // Check GH_CONFIG_DIR environment variable first
    if let Ok(config_dir) = env.var("GH_CONFIG_DIR") {
        return Some(PathBuf::from(config_dir));
    }

    // Fall back to default locations
    if let Ok(home_dir) = env.var("HOME") {
        Some(PathBuf::from(home_dir).join(".config").join("gh"))
    } else if let Ok(user_profile) = env.var("USERPROFILE") {
        // Windows fallback
        Some(
            PathBuf::from(user_profile)
                .join("AppData")
                .join("Roaming")
                .join("GitHub CLI"),
        )
    } else {
        None
    }
}

fn get_netrc_path_with_env(env: &impl EnvProvider) -> Option<PathBuf> {
    if let Ok(home_dir) = env.var("HOME") {
        Some(PathBuf::from(home_dir).join(".netrc"))
    } else if let Ok(user_profile) = env.var("USERPROFILE") {
        // Windows: try both _netrc and .netrc
        let netrc_path = PathBuf::from(&user_profile).join("_netrc");
        if netrc_path.exists() {
            Some(netrc_path)
        } else {
            Some(PathBuf::from(user_profile).join(".netrc"))
        }
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::MockEnvProvider;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_extract_host_from_url() {
        assert_eq!(extract_host_from_url("https://github.com"), "github.com");
        assert_eq!(
            extract_host_from_url("https://github.enterprise.com"),
            "github.enterprise.com"
        );
        assert_eq!(
            extract_host_from_url("https://ghe.company.internal"),
            "ghe.company.internal"
        );
    }

    #[test]
    fn test_gh_token_github_com() {
        let temp_dir = TempDir::new().unwrap();
        let hosts_file = temp_dir.path().join("hosts.yml");

        let hosts_content = r#"
github.com:
    user: testuser
    oauth_token: ghp_test_token_123
    git_protocol: https
"#;
        fs::write(&hosts_file, hosts_content).unwrap();

        let mut mock_env = MockEnvProvider::new();
        mock_env
            .expect_var()
            .with(mockall::predicate::eq("GH_CONFIG_DIR"))
            .times(1)
            .returning(move |_| Ok(temp_dir.path().to_string_lossy().to_string()));

        let token = get_gh_token_with_env("https://github.com", &mock_env).unwrap();
        assert_eq!(token, "ghp_test_token_123");
    }

    #[test]
    fn test_netrc_parsing() {
        let temp_dir = TempDir::new().unwrap();
        let netrc_file = temp_dir.path().join(".netrc");

        let netrc_content = r#"
machine github.com
login testuser
password ghp_test_netrc_token

machine api.github.com
login testuser  
password ghp_api_token_456
"#;
        fs::write(&netrc_file, netrc_content).unwrap();

        let mut mock_env = MockEnvProvider::new();
        mock_env
            .expect_var()
            .with(mockall::predicate::eq("HOME"))
            .times(1)
            .returning(move |_| Ok(temp_dir.path().to_string_lossy().to_string()));

        let token = get_netrc_token_with_env("https://github.com", &mock_env).unwrap();
        assert_eq!(token, "ghp_test_netrc_token");
    }

    #[tokio::test]
    async fn test_environment_variable_priority() {
        // Mock environment with GITHUB_TOKEN - this should have highest priority
        let mut mock_env = MockEnvProvider::new();
        mock_env
            .expect_var()
            .with(mockall::predicate::eq("GITHUB_TOKEN"))
            .times(1)
            .returning(|_| Ok("ghp_env_token".to_string()));

        let token = get_token("https://github.com", &mock_env);
        let client = create_authenticated_client("https://github.com", token);
        assert!(client.is_ok());
    }
}
