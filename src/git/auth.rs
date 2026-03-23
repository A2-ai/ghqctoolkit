use crate::auth::{extract_host_from_base_url, load_token, validate_github_token};
use crate::utils::EnvProvider;
use octocrab::Octocrab;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

#[derive(thiserror::Error, Debug)]
pub enum AuthError {
    #[error("Failed to build octocrab client: {0}")]
    ClientBuild(#[from] octocrab::Error),
    #[error(
        "No authentication found. Try: 'ghqc auth login', GITHUB_TOKEN, 'gh auth login', or git credential manager"
    )]
    NoAuth,
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON parsing error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("YAML parsing error: {0}")]
    Yaml(#[from] serde_yaml::Error),
}

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const READ_TIMEOUT: Duration = Duration::from_secs(25);

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
    let stored_token = match load_token(base_url) {
        Ok(token) => token,
        Err(e) => {
            log::debug!("Failed to read ghqc auth store: {}", e);
            None
        }
    };
    get_token_with_stored(base_url, env, stored_token)
}

fn get_token_with_stored(
    base_url: &str,
    env: &impl EnvProvider,
    stored_token: Option<String>,
) -> Option<String> {
    // Try authentication sources in priority order (similar to gitcreds R package)

    // 1. ghqc auth store
    log::debug!("Trying from ghqc auth store");
    if let Some(token) = stored_token {
        log::debug!("Using ghqc stored credentials");
        return Some(token);
    }

    // 2. GITHUB_TOKEN environment variable
    log::debug!("Trying from GITHUB_TOKEN");
    if let Ok(token) = env.var("GITHUB_TOKEN") {
        log::debug!("Found GITHUB_TOKEN environment variable");
        if let Some(validated_token) = validate_github_token(&token) {
            log::debug!("Using GITHUB_TOKEN environment variable");
            return Some(validated_token);
        }
    }

    // 3. gh CLI active authentication (gh auth token)
    log::debug!("Trying from gh auth token command");
    if let Some(token) = get_gh_auth_token(base_url) {
        log::debug!("Using gh CLI active token");
        return Some(token);
    }

    // 4. gh CLI stored authentication (config files)
    log::debug!("Trying from gh cli stored authentication");
    if let Some(token) = get_gh_token_with_env(base_url, env) {
        log::debug!("Using gh CLI stored credentials");
        return Some(token);
    }

    // 5. Git credential manager (git credential fill)
    log::debug!("Trying from git credential manager");
    if let Some(token) = get_git_credential_token(base_url) {
        log::debug!("Using git credential manager");
        return Some(token);
    }

    // 6. .netrc file
    log::debug!("Trying from .netrc file");
    if let Some(token) = get_netrc_token_with_env(base_url, env) {
        log::debug!("Using .netrc file credentials");
        return Some(token);
    }

    log::warn!("No authentication found. API access will be limited to public repositories");

    None
}

fn build_client_with_token(base_url: &str, token: String) -> Result<Octocrab, AuthError> {
    log::debug!("Creating Octocrab client (assuming proper runtime context)");
    let builder = Octocrab::builder()
        .set_connect_timeout(Some(CONNECT_TIMEOUT))
        .set_read_timeout(Some(READ_TIMEOUT))
        .personal_token(token);
    if base_url == "https://github.com" {
        builder.build()
    } else {
        builder
            .base_uri(format!("{}/api/v3", base_url))
            .and_then(|b| b.build())
    }
    .map_err(AuthError::ClientBuild)
}

fn build_unauthenticated_client(base_url: &str) -> Result<Octocrab, AuthError> {
    log::debug!("Creating unauthenticated Octocrab client (assuming proper runtime context)");
    let builder = Octocrab::builder()
        .set_connect_timeout(Some(CONNECT_TIMEOUT))
        .set_read_timeout(Some(READ_TIMEOUT));
    if base_url == "https://github.com" {
        builder.build()
    } else {
        builder
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

    if let Some(host_config) = hosts.get(host.as_str()) {
        if let Some(oauth_token) = host_config.get("oauth_token") {
            if let Some(token_str) = oauth_token.as_str() {
                return validate_github_token(token_str);
            }
        }
    }

    None
}

fn get_git_credential_token(base_url: &str) -> Option<String> {
    let host = extract_host_from_url(base_url);

    // Use git credential fill to get stored credentials
    let mut cmd = Command::new("git");
    cmd.args(&["credential", "fill"])
        // Force credential lookup to remain non-interactive even if a helper
        // would otherwise try to prompt or launch a GUI.
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GCM_INTERACTIVE", "never")
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
            return validate_github_token(password);
        }
    }

    None
}

fn get_gh_auth_token(base_url: &str) -> Option<String> {
    let host = extract_host_from_url(base_url);

    // Use gh auth token --hostname <host> to get the active token
    let mut cmd = Command::new("gh");
    cmd.args(["auth", "token", "--hostname", host.as_str()])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    // Add extra error handling to prevent crashes
    let child = match cmd.spawn() {
        Ok(child) => child,
        Err(e) => {
            log::debug!("Failed to spawn gh auth token process: {}", e);
            return None;
        }
    };

    let output = match child.wait_with_output() {
        Ok(output) => output,
        Err(e) => {
            log::debug!("Failed to wait for gh auth token output: {}", e);
            return None;
        }
    };

    if !output.status.success() {
        log::debug!(
            "gh auth token command failed with status: {}",
            output.status
        );
        return None;
    }

    let output_str = String::from_utf8_lossy(&output.stdout);
    let token = output_str.trim();

    if token.is_empty() {
        log::debug!("gh auth token returned empty token");
        return None;
    }

    validate_github_token(token)
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
                        if let Some(validated_token) = validate_github_token(password) {
                            return Some(validated_token);
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

fn extract_host_from_url(base_url: &str) -> String {
    extract_host_from_base_url(base_url).unwrap_or_else(|_| {
        base_url
            .strip_prefix("https://")
            .unwrap_or(base_url)
            .to_string()
    })
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
    oauth_token: ghp_test_token_1234567890123456789012345678901234567890
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
        assert_eq!(
            token,
            "ghp_test_token_1234567890123456789012345678901234567890"
        );
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

    #[test]
    fn test_ghqc_store_priority_over_environment_variable() {
        let mut mock_env = MockEnvProvider::new();
        mock_env
            .expect_var()
            .with(mockall::predicate::eq("GITHUB_TOKEN"))
            .times(0);

        let token = get_token_with_stored(
            "https://github.com",
            &mock_env,
            Some("ghp_stored_token_1234567890123456789012345678901234567890".to_string()),
        );
        assert_eq!(
            token,
            Some("ghp_stored_token_1234567890123456789012345678901234567890".to_string())
        );
    }

    #[test]
    fn test_environment_variable_priority_without_store() {
        let mut mock_env = MockEnvProvider::new();
        mock_env
            .expect_var()
            .with(mockall::predicate::eq("GITHUB_TOKEN"))
            .times(1)
            .returning(|_| {
                Ok("ghp_valid_env_token_1234567890123456789012345678901234567890".to_string())
            });

        let token = get_token_with_stored("https://github.com", &mock_env, None);
        assert_eq!(
            token,
            Some("ghp_valid_env_token_1234567890123456789012345678901234567890".to_string())
        );
    }

    #[test]
    fn test_validate_github_token() {
        // Valid tokens
        assert_eq!(
            validate_github_token("ghp_1234567890123456789012345678901234567890"),
            Some("ghp_1234567890123456789012345678901234567890".to_string())
        );
        assert_eq!(
            validate_github_token("github_pat_1234567890123456789012345678901"),
            Some("github_pat_1234567890123456789012345678901".to_string())
        );
        assert_eq!(
            validate_github_token("gho_1234567890123456789012345678901234567890"),
            Some("gho_1234567890123456789012345678901234567890".to_string())
        );
        assert_eq!(
            validate_github_token("ghu_1234567890123456789012345678901234567890"),
            Some("ghu_1234567890123456789012345678901234567890".to_string())
        );
        assert_eq!(
            validate_github_token("ghs_1234567890123456789012345678901234567890"),
            Some("ghs_1234567890123456789012345678901234567890".to_string())
        );
        assert_eq!(
            validate_github_token("ghr_1234567890123456789012345678901234567890"),
            Some("ghr_1234567890123456789012345678901234567890".to_string())
        );

        // Invalid tokens - too short
        assert_eq!(validate_github_token("ghp_123"), None);
        assert_eq!(validate_github_token("github_pat_123"), None);

        // Empty token
        assert_eq!(validate_github_token(""), None);
    }
}
