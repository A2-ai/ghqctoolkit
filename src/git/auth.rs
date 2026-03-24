use crate::auth::{AuthStore, extract_host_from_base_url, validate_github_token};
use crate::utils::EnvProvider;
use octocrab::Octocrab;
use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const READ_TIMEOUT: Duration = Duration::from_secs(25);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AuthSourceKind {
    GhqcStore,
    GithubTokenEnv,
    GhActiveToken,
    GhStoredAuth,
    GitCredentialManager,
    Netrc,
}

impl fmt::Display for AuthSourceKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Self::GhqcStore => "ghqc auth store",
                Self::GithubTokenEnv => "GITHUB_TOKEN",
                Self::GhActiveToken => "gh auth token",
                Self::GhStoredAuth => "gh stored auth",
                Self::GitCredentialManager => "git credential manager",
                Self::Netrc => ".netrc",
            }
        )
    }
}

#[derive(Debug, Clone)]
pub struct AuthSources(HashMap<AuthSourceKind, String>);

impl AuthSources {
    pub fn new(base_url: &str, env: &impl EnvProvider, auth_store: Option<&AuthStore>) -> Self {
        let mut res = HashMap::new();

        let host = extract_host_from_url(base_url);
        if let Some(store) = auth_store {
            if let Some(token) = store.token(&host) {
                res.insert(AuthSourceKind::GhqcStore, token.to_string());
            }
        }

        if let Ok(token) = env.var("GITHUB_TOKEN") {
            res.insert(AuthSourceKind::GithubTokenEnv, token);
        }

        if let Some(token) = get_gh_auth_token(base_url) {
            res.insert(AuthSourceKind::GhActiveToken, token);
        }

        if let Some(token) = get_gh_token_with_env(base_url, env) {
            res.insert(AuthSourceKind::GhStoredAuth, token);
        }

        if let Some(token) = get_git_credential_token(base_url) {
            res.insert(AuthSourceKind::GitCredentialManager, token);
        }

        if let Some(token) = get_netrc_token_with_env(base_url, env) {
            res.insert(AuthSourceKind::Netrc, token);
        }

        AuthSources(res)
    }

    pub fn token(&self) -> Option<&str> {
        self.sorted().first().map(|(kind, token)| {
            log::debug!("Using authentication from {kind}");
            token.as_str()
        })
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn client(&self, base_url: &str) -> Result<Octocrab, AuthError> {
        log::debug!("Creating Octocrab client");
        let mut builder = Octocrab::builder()
            .set_connect_timeout(Some(CONNECT_TIMEOUT))
            .set_read_timeout(Some(READ_TIMEOUT));
        if let Some(token) = self.token() {
            builder = builder.personal_token(token.to_string());
        } else {
            log::warn!(
                "No authentication found. API access will be limited to public repositories"
            );
        }

        if base_url == "https://github.com" {
            builder.build()
        } else {
            builder
                .base_uri(format!("{base_url}/api/v3"))
                .and_then(|b| b.build())
        }
        .map_err(AuthError::ClientBuild)
    }

    pub fn sorted(&self) -> Vec<(&AuthSourceKind, &String)> {
        let mut v = self.0.iter().collect::<Vec<_>>();
        v.sort_by(|(a, _), (b, _)| a.cmp(b));
        v
    }

    /// All auth source kinds in priority order, with their token if available.
    pub fn all_by_priority(&self) -> Vec<(AuthSourceKind, Option<&str>)> {
        use AuthSourceKind::*;
        [
            GhqcStore,
            GithubTokenEnv,
            GhActiveToken,
            GhStoredAuth,
            GitCredentialManager,
            Netrc,
        ]
        .into_iter()
        .map(|kind| {
            let token = self.0.get(&kind).map(|s| s.as_str());
            (kind, token)
        })
        .collect()
    }
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

    fn make_store(tokens: &[(&str, &str)]) -> crate::auth::AuthStore {
        use crate::auth::AuthToken;
        use std::collections::HashMap;
        let tokens = tokens
            .iter()
            .map(|(host, token)| {
                (
                    host.to_string(),
                    AuthToken {
                        host: host.to_string(),
                        token: token.to_string(),
                    },
                )
            })
            .collect::<HashMap<_, _>>();
        crate::auth::AuthStore {
            root: std::path::PathBuf::new(),
            master_key: vec![],
            tokens,
        }
    }

    #[test]
    fn ghqc_store_selected_over_github_token_env() {
        let store = make_store(&[(
            "github.com",
            "ghp_stored_token_1234567890123456789012345678901234567890",
        )]);
        let mut mock_env = MockEnvProvider::new();
        mock_env
            .expect_var()
            .with(mockall::predicate::eq("GITHUB_TOKEN"))
            .times(1)
            .returning(
                |_| Ok("ghp_env_token_1234567890123456789012345678901234567890".to_string()),
            );
        mock_env
            .expect_var()
            .with(mockall::predicate::eq("GH_CONFIG_DIR"))
            .times(1)
            .returning(|_| Err(std::env::VarError::NotPresent));
        mock_env
            .expect_var()
            .with(mockall::predicate::eq("HOME"))
            .times(2)
            .returning(|_| Err(std::env::VarError::NotPresent));
        mock_env
            .expect_var()
            .with(mockall::predicate::eq("USERPROFILE"))
            .times(2)
            .returning(|_| Err(std::env::VarError::NotPresent));

        let sources = AuthSources::new("https://github.com", &mock_env, Some(&store));
        assert_eq!(
            sources.token(),
            Some("ghp_stored_token_1234567890123456789012345678901234567890")
        );
        assert_eq!(
            sources.sorted().first().map(|(k, _)| *k),
            Some(&AuthSourceKind::GhqcStore)
        );
    }

    #[test]
    fn github_token_env_used_when_no_store() {
        let mut mock_env = MockEnvProvider::new();
        mock_env
            .expect_var()
            .with(mockall::predicate::eq("GITHUB_TOKEN"))
            .times(1)
            .returning(
                |_| Ok("ghp_env_token_1234567890123456789012345678901234567890".to_string()),
            );
        mock_env
            .expect_var()
            .with(mockall::predicate::eq("GH_CONFIG_DIR"))
            .times(1)
            .returning(|_| Err(std::env::VarError::NotPresent));
        mock_env
            .expect_var()
            .with(mockall::predicate::eq("HOME"))
            .times(2)
            .returning(|_| Err(std::env::VarError::NotPresent));
        mock_env
            .expect_var()
            .with(mockall::predicate::eq("USERPROFILE"))
            .times(2)
            .returning(|_| Err(std::env::VarError::NotPresent));

        let sources = AuthSources::new("https://github.com", &mock_env, None);
        assert_eq!(
            sources.token(),
            Some("ghp_env_token_1234567890123456789012345678901234567890")
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
