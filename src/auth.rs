use base64::Engine;
use base64::prelude::BASE64_URL_SAFE_NO_PAD;
use etcetera::BaseStrategy;
use openssl::rand::rand_bytes;
use openssl::symm::{Cipher, Crypter, Mode};
use owo_colors::OwoColorize;
use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

const MASTER_KEY_LEN: usize = 32;
const NONCE_LEN: usize = 12;
const TAG_LEN: usize = 16;

#[derive(thiserror::Error, Debug)]
pub enum AuthStoreError {
    #[error("Failed to determine auth store directory: {0}")]
    StoreDir(String),
    #[error("Failed to create auth store directory: {0}")]
    CreateDir(#[from] std::io::Error),
    #[error("Invalid host '{0}'. Expected a hostname or https:// URL")]
    InvalidHost(String),
    #[error("Invalid token format")]
    InvalidToken,
    #[error("Failed to encrypt stored token: {0}")]
    Encrypt(openssl::error::ErrorStack),
    #[error("Failed to decrypt stored token: {0}")]
    Decrypt(openssl::error::ErrorStack),
}

#[derive(Debug, Clone)]
pub struct AuthToken {
    pub host: String,
    pub(crate) token: String,
}

impl fmt::Display for AuthToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({})", self.host, preview_token(&self.token))
    }
}

#[derive(Debug, Clone)]
pub struct AuthStore {
    pub root: PathBuf,
    pub(crate) master_key: Vec<u8>,
    pub(crate) tokens: HashMap<String, AuthToken>,
}

impl AuthStore {
    pub fn new(dir: Option<impl AsRef<Path>>) -> Result<Self, AuthStoreError> {
        let root = if let Some(dir) = dir {
            dir.as_ref().to_path_buf()
        } else {
            let strategy = etcetera::choose_base_strategy()
                .map_err(|e| AuthStoreError::StoreDir(e.to_string()))?;
            strategy.data_dir().join("ghqc").join("auth")
        };

        ensure_dir(&root)?;

        let key_path = root.join("master.key");
        let key = if key_path.exists() {
            fs::read(&key_path)?
        } else {
            let mut key = vec![0u8; MASTER_KEY_LEN];
            rand_bytes(&mut key).map_err(AuthStoreError::Encrypt)?;
            fs::write(&key_path, &key)?;
            set_user_only_permissions(&key_path)?;
            key
        };

        Ok(Self {
            root,
            master_key: key,
            tokens: HashMap::new(),
        })
    }

    fn hosts_dir(&self) -> PathBuf {
        self.root.join("hosts")
    }

    fn token_path(&self, host: &str) -> PathBuf {
        let encoded = BASE64_URL_SAFE_NO_PAD.encode(host);
        self.hosts_dir().join(format!("{encoded}.enc"))
    }

    /// Populate `tokens` from disk. Call before using `token()` or `display_with_selected()`.
    pub fn load(&mut self) {
        let load_token = |path: &Path| -> Option<String> {
            let data = fs::read(path).ok()?;
            if data.len() < NONCE_LEN + TAG_LEN {
                return None;
            }

            let (nonce, rest) = data.split_at(NONCE_LEN);
            let (tag, ciphertext) = rest.split_at(TAG_LEN);
            let plaintext = match decrypt_bytes(&self.master_key, nonce, tag, ciphertext) {
                Ok(t) => t,
                Err(e) => {
                    log::error!("Failed to decrypt file at {}: {e}", path.display());
                    return None;
                }
            };

            match String::from_utf8(plaintext) {
                Ok(s) => validate_github_token(&s),
                Err(_) => {
                    log::error!("Invalid UTF-8 token at {}", path.display());
                    None
                }
            }
        };

        let read_dir = match fs::read_dir(self.hosts_dir()) {
            Ok(d) => d,
            Err(e) => {
                log::debug!(
                    "Failed to read {}: {e}. Skipping auth store loading...",
                    self.hosts_dir().display()
                );
                return;
            }
        };

        self.tokens = read_dir
            .into_iter()
            .filter_map(Result::ok)
            .filter(|entry| entry.path().extension().and_then(|s| s.to_str()) == Some("enc"))
            .filter_map(|entry| {
                let mut path = entry.path();
                path.set_extension("");
                path.file_name().and_then(|s| s.to_str()).and_then(|s| {
                    BASE64_URL_SAFE_NO_PAD
                        .decode(s)
                        .ok()
                        .and_then(|b| String::from_utf8(b).ok())
                        .map(|host| (entry.path(), host))
                })
            })
            .filter_map(|(path, host)| load_token(&path).map(|t| (host, t)))
            .map(|(host, token)| (host.clone(), AuthToken { host, token }))
            .collect();
    }

    /// Returns the stored token for `host`, if loaded. Call `load()` first.
    pub fn token(&self, host: &str) -> Option<&str> {
        self.tokens.get(host).map(|a| a.token.as_str())
    }

    /// Validates and encrypts `token` to disk for `host`.
    pub fn save_token(&self, host: &str, token: &str) -> Result<(), AuthStoreError> {
        let validated = validate_github_token(token).ok_or(AuthStoreError::InvalidToken)?;

        let hosts_dir = self.hosts_dir();
        if !hosts_dir.exists() {
            fs::create_dir_all(&hosts_dir)?;
        }

        let mut nonce = [0u8; NONCE_LEN];
        rand_bytes(&mut nonce).map_err(AuthStoreError::Encrypt)?;

        let (cipher_text, tag) = encrypt_bytes(&self.master_key, &nonce, validated.as_bytes())
            .map_err(AuthStoreError::Encrypt)?;

        let mut payload = Vec::with_capacity(NONCE_LEN + TAG_LEN + cipher_text.len());
        payload.extend_from_slice(&nonce);
        payload.extend_from_slice(&tag);
        payload.extend_from_slice(&cipher_text);

        fs::write(self.token_path(host), payload)?;
        Ok(())
    }

    /// Removes the stored token for `host`. Returns `true` if a file was deleted.
    pub fn delete_token(&self, host: &str) -> Result<bool, AuthStoreError> {
        let token_path = self.token_path(host);
        if !token_path.exists() {
            log::debug!(
                "Token path for {} ({}) does not exist",
                host,
                token_path.display()
            );
            return Ok(false);
        }
        log::debug!("Removing token for {} at {}", host, token_path.display());
        fs::remove_file(token_path)?;
        Ok(true)
    }

    /// Returns a formatted string of all stored hosts, sorted alphabetically.
    /// The host matching `selected` is marked with `▶`. Call `load()` first.
    pub fn display_with_selected(&self, selected: Option<&str>) -> String {
        if self.tokens.is_empty() {
            return "none".to_string();
        }

        let mut sorted: Vec<&AuthToken> = self.tokens.values().collect();
        sorted.sort_by(|a, b| a.host.cmp(&b.host));

        sorted
            .iter()
            .map(|t| {
                if selected == Some(t.host.as_str()) {
                    format!(
                        "{} {} ({})",
                        "▶".green(),
                        t.host.bold(),
                        preview_token(&t.token)
                    )
                    // "▶ ".green().to_string()
                } else {
                    format!("  {t}")
                    // "  ".to_string()
                }
                // format!("{marker}{t}")
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

pub fn validate_github_token(token: &str) -> Option<String> {
    let token = token.trim();
    if token.is_empty() || token.contains(char::is_whitespace) {
        return None;
    }

    let has_known_prefix = token.starts_with("ghp_")
        || token.starts_with("github_pat_")
        || token.starts_with("gho_")
        || token.starts_with("ghu_")
        || token.starts_with("ghs_")
        || token.starts_with("ghr_");

    if has_known_prefix && token.len() >= 20 {
        return Some(token.to_string());
    }

    if token.len() >= 20
        && token
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        log::debug!(
            "Accepting non-prefixed token ({} chars) - likely GHE classic PAT",
            token.len()
        );
        return Some(token.to_string());
    }

    log::debug!(
        "Rejected token: {} chars, starts with: {}",
        token.len(),
        token.chars().take(6).collect::<String>()
    );
    None
}

pub fn canonicalize_base_url(input: &str) -> Result<String, AuthStoreError> {
    let trimmed = input.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Err(AuthStoreError::InvalidHost(input.to_string()));
    }

    let with_scheme = if trimmed.starts_with("https://") {
        trimmed.to_string()
    } else if trimmed.starts_with("http://") {
        return Err(AuthStoreError::InvalidHost(input.to_string()));
    } else if trimmed.contains("://") {
        return Err(AuthStoreError::InvalidHost(input.to_string()));
    } else {
        format!("https://{trimmed}")
    };

    let parsed = url::Url::parse(&with_scheme)
        .map_err(|_| AuthStoreError::InvalidHost(input.to_string()))?;
    if parsed.scheme() != "https" || parsed.host_str().is_none() {
        return Err(AuthStoreError::InvalidHost(input.to_string()));
    }

    Ok(format!(
        "https://{}",
        parsed
            .host_str()
            .ok_or_else(|| AuthStoreError::InvalidHost(input.to_string()))?
    ))
}

pub fn extract_host_from_base_url(base_url: &str) -> Result<String, AuthStoreError> {
    let normalized = canonicalize_base_url(base_url)?;
    Ok(normalized.trim_start_matches("https://").to_string())
}

pub fn token_page_url(base_url: &str) -> Result<String, AuthStoreError> {
    Ok(format!(
        "{}/settings/tokens/new",
        canonicalize_base_url(base_url)?
    ))
}

fn ensure_dir(path: &Path) -> Result<(), AuthStoreError> {
    fs::create_dir_all(path)?;
    set_user_only_permissions(path)?;
    Ok(())
}

fn set_user_only_permissions(path: &Path) -> Result<(), AuthStoreError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mode = if path.is_dir() { 0o700 } else { 0o600 };
        fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
    }
    Ok(())
}

fn encrypt_bytes(
    key: &[u8],
    nonce: &[u8],
    plaintext: &[u8],
) -> std::result::Result<(Vec<u8>, [u8; TAG_LEN]), openssl::error::ErrorStack> {
    let cipher = Cipher::aes_256_gcm();
    let mut crypter = Crypter::new(cipher, Mode::Encrypt, key, Some(nonce))?;
    crypter.pad(false);

    let mut ciphertext = vec![0; plaintext.len() + cipher.block_size()];
    let mut count = crypter.update(plaintext, &mut ciphertext)?;
    count += crypter.finalize(&mut ciphertext[count..])?;
    ciphertext.truncate(count);

    let mut tag = [0u8; TAG_LEN];
    crypter.get_tag(&mut tag)?;
    Ok((ciphertext, tag))
}

fn decrypt_bytes(
    key: &[u8],
    nonce: &[u8],
    tag: &[u8],
    ciphertext: &[u8],
) -> std::result::Result<Vec<u8>, openssl::error::ErrorStack> {
    let cipher = Cipher::aes_256_gcm();
    let mut crypter = Crypter::new(cipher, Mode::Decrypt, key, Some(nonce))?;
    crypter.pad(false);
    crypter.set_tag(tag)?;

    let mut plaintext = vec![0; ciphertext.len() + cipher.block_size()];
    let mut count = crypter.update(ciphertext, &mut plaintext)?;
    count += crypter.finalize(&mut plaintext[count..])?;
    plaintext.truncate(count);
    Ok(plaintext)
}

pub(crate) fn preview_token(token: &str) -> String {
    if token.len() <= 8 {
        return "*".repeat(token.len());
    }
    format!("{}...{}", &token[..4], &token[token.len() - 4..])
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn canonicalizes_hosts() {
        assert_eq!(
            canonicalize_base_url("github.com").unwrap(),
            "https://github.com"
        );
        assert_eq!(
            canonicalize_base_url("https://ghe.example.com/").unwrap(),
            "https://ghe.example.com"
        );
        assert!(canonicalize_base_url("http://github.com").is_err());
    }

    #[test]
    fn store_roundtrip_is_host_scoped() {
        let temp = TempDir::new().unwrap();

        let store = AuthStore::new(Some(temp.path())).unwrap();
        store
            .save_token(
                "github.com",
                "ghp_token_1234567890123456789012345678901234567890",
            )
            .unwrap();
        store
            .save_token(
                "ghe.example.com",
                "ghp_token_abcdefghijabcdefghijabcdefghijabcdefghij",
            )
            .unwrap();

        let mut store = AuthStore::new(Some(temp.path())).unwrap();
        store.load();

        assert_eq!(
            store.token("github.com"),
            Some("ghp_token_1234567890123456789012345678901234567890")
        );
        assert_eq!(
            store.token("ghe.example.com"),
            Some("ghp_token_abcdefghijabcdefghijabcdefghijabcdefghij")
        );
    }

    #[test]
    fn delete_is_host_scoped() {
        let temp = TempDir::new().unwrap();
        let store = AuthStore::new(Some(temp.path())).unwrap();
        store
            .save_token(
                "github.com",
                "ghp_token_1234567890123456789012345678901234567890",
            )
            .unwrap();
        store
            .save_token(
                "ghe.example.com",
                "ghp_token_abcdefghijabcdefghijabcdefghijabcdefghij",
            )
            .unwrap();

        assert!(store.delete_token("github.com").unwrap());

        let mut store = AuthStore::new(Some(temp.path())).unwrap();
        store.load();
        assert_eq!(store.token("github.com"), None);
        assert!(store.token("ghe.example.com").is_some());
    }

    #[test]
    fn corrupted_ciphertext_skipped_on_load() {
        let temp = TempDir::new().unwrap();
        let store = AuthStore::new(Some(temp.path())).unwrap();
        store
            .save_token(
                "github.com",
                "ghp_token_1234567890123456789012345678901234567890",
            )
            .unwrap();

        let encoded = BASE64_URL_SAFE_NO_PAD.encode("github.com");
        let token_path = temp.path().join("hosts").join(format!("{encoded}.enc"));
        fs::write(token_path, b"broken").unwrap();

        let mut store = AuthStore::new(Some(temp.path())).unwrap();
        store.load();
        assert_eq!(store.token("github.com"), None);
    }

    #[test]
    fn missing_host_returns_none() {
        let temp = TempDir::new().unwrap();
        let mut store = AuthStore::new(Some(temp.path())).unwrap();
        store.load();
        assert_eq!(store.token("github.com"), None);
        assert!(!store.delete_token("github.com").unwrap());
    }

    #[test]
    fn display_with_selected_marks_active_host() {
        let temp = TempDir::new().unwrap();
        let store = AuthStore::new(Some(temp.path())).unwrap();
        store
            .save_token(
                "ghe.example.com",
                "ghp_token_abcdefghijabcdefghijabcdefghijabcdefghij",
            )
            .unwrap();
        store
            .save_token(
                "github.com",
                "ghp_token_1234567890123456789012345678901234567890",
            )
            .unwrap();

        let mut store = AuthStore::new(Some(temp.path())).unwrap();
        store.load();

        let output = store.display_with_selected(Some("github.com"));
        let lines: Vec<&str> = output.lines().collect();
        assert_eq!(lines.len(), 2);
        // sorted: ghe.example.com first, github.com second
        assert!(lines[0].starts_with("  "));
        assert!(lines[1].starts_with("\u{1b}[32m▶\u{1b}[39m \u{1b}[1mgithub.com\u{1b}[0m"));
    }

    #[test]
    fn display_with_selected_none_shows_all_unmarked() {
        let temp = TempDir::new().unwrap();
        let store = AuthStore::new(Some(temp.path())).unwrap();
        store
            .save_token(
                "github.com",
                "ghp_token_1234567890123456789012345678901234567890",
            )
            .unwrap();

        let mut store = AuthStore::new(Some(temp.path())).unwrap();
        store.load();

        let output = store.display_with_selected(None);
        assert!(output.starts_with("  "));
        assert!(output.contains("github.com"));
    }
}
