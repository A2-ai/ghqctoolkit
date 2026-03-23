use base64::Engine;
use etcetera::BaseStrategy;
use openssl::rand::rand_bytes;
use openssl::symm::{Cipher, Crypter, Mode};
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

pub fn auth_store_root() -> Result<PathBuf, AuthStoreError> {
    let strategy =
        etcetera::choose_base_strategy().map_err(|e| AuthStoreError::StoreDir(e.to_string()))?;
    Ok(strategy.data_dir().join("ghqc").join("auth"))
}

pub fn load_token(base_url: &str) -> Result<Option<String>, AuthStoreError> {
    load_token_at(&auth_store_root()?, base_url)
}

pub fn save_token(base_url: &str, token: &str) -> Result<(), AuthStoreError> {
    let validated = validate_github_token(token).ok_or(AuthStoreError::InvalidToken)?;
    save_token_at(&auth_store_root()?, base_url, &validated)
}

pub fn delete_token(base_url: &str) -> Result<bool, AuthStoreError> {
    delete_token_at(&auth_store_root()?, base_url)
}

pub fn token_page_url(base_url: &str) -> Result<String, AuthStoreError> {
    Ok(format!(
        "{}/settings/tokens/new",
        canonicalize_base_url(base_url)?
    ))
}

pub fn load_token_at(root: &Path, base_url: &str) -> Result<Option<String>, AuthStoreError> {
    let path = token_file_path(root, base_url)?;
    if !path.exists() {
        return Ok(None);
    }

    let data = fs::read(path)?;
    if data.len() < NONCE_LEN + TAG_LEN {
        return Err(AuthStoreError::Decrypt(openssl::error::ErrorStack::get()));
    }

    let (nonce, rest) = data.split_at(NONCE_LEN);
    let (tag, ciphertext) = rest.split_at(TAG_LEN);
    let key = load_or_create_master_key(root)?;

    let plaintext = decrypt_bytes(&key, nonce, tag, ciphertext).map_err(AuthStoreError::Decrypt)?;
    let token = String::from_utf8(plaintext).map_err(|_| AuthStoreError::InvalidToken)?;
    Ok(validate_github_token(&token))
}

pub fn save_token_at(root: &Path, base_url: &str, token: &str) -> Result<(), AuthStoreError> {
    ensure_dir(root)?;
    ensure_dir(&hosts_dir(root))?;

    let key = load_or_create_master_key(root)?;
    let mut nonce = [0u8; NONCE_LEN];
    rand_bytes(&mut nonce).map_err(AuthStoreError::Encrypt)?;
    let (ciphertext, tag) =
        encrypt_bytes(&key, &nonce, token.as_bytes()).map_err(AuthStoreError::Encrypt)?;

    let mut payload = Vec::with_capacity(NONCE_LEN + TAG_LEN + ciphertext.len());
    payload.extend_from_slice(&nonce);
    payload.extend_from_slice(&tag);
    payload.extend_from_slice(&ciphertext);

    let path = token_file_path(root, base_url)?;
    fs::write(path, payload)?;
    Ok(())
}

pub fn delete_token_at(root: &Path, base_url: &str) -> Result<bool, AuthStoreError> {
    let path = token_file_path(root, base_url)?;
    if !path.exists() {
        return Ok(false);
    }
    fs::remove_file(path)?;
    Ok(true)
}

fn token_file_path(root: &Path, base_url: &str) -> Result<PathBuf, AuthStoreError> {
    let host = extract_host_from_base_url(base_url)?;
    let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(host);
    Ok(hosts_dir(root).join(format!("{encoded}.enc")))
}

fn hosts_dir(root: &Path) -> PathBuf {
    root.join("hosts")
}

fn master_key_path(root: &Path) -> PathBuf {
    root.join("master.key")
}

fn load_or_create_master_key(root: &Path) -> Result<Vec<u8>, AuthStoreError> {
    ensure_dir(root)?;
    let key_path = master_key_path(root);
    if key_path.exists() {
        return fs::read(key_path).map_err(AuthStoreError::CreateDir);
    }

    let mut key = vec![0u8; MASTER_KEY_LEN];
    rand_bytes(&mut key).map_err(AuthStoreError::Encrypt)?;
    fs::write(&key_path, &key)?;
    set_user_only_permissions(&key_path)?;
    Ok(key)
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

        save_token_at(
            temp.path(),
            "https://github.com",
            "ghp_token_1234567890123456789012345678901234567890",
        )
        .unwrap();
        save_token_at(
            temp.path(),
            "https://ghe.example.com",
            "ghp_token_abcdefghijabcdefghijabcdefghijabcdefghij",
        )
        .unwrap();

        assert_eq!(
            load_token_at(temp.path(), "github.com").unwrap(),
            Some("ghp_token_1234567890123456789012345678901234567890".to_string())
        );
        assert_eq!(
            load_token_at(temp.path(), "https://ghe.example.com").unwrap(),
            Some("ghp_token_abcdefghijabcdefghijabcdefghijabcdefghij".to_string())
        );
    }

    #[test]
    fn delete_is_host_scoped() {
        let temp = TempDir::new().unwrap();
        save_token_at(
            temp.path(),
            "github.com",
            "ghp_token_1234567890123456789012345678901234567890",
        )
        .unwrap();
        save_token_at(
            temp.path(),
            "ghe.example.com",
            "ghp_token_abcdefghijabcdefghijabcdefghijabcdefghij",
        )
        .unwrap();

        assert!(delete_token_at(temp.path(), "github.com").unwrap());
        assert_eq!(load_token_at(temp.path(), "github.com").unwrap(), None);
        assert!(
            load_token_at(temp.path(), "ghe.example.com")
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn corrupted_ciphertext_fails_cleanly() {
        let temp = TempDir::new().unwrap();
        save_token_at(
            temp.path(),
            "github.com",
            "ghp_token_1234567890123456789012345678901234567890",
        )
        .unwrap();

        let token_path = token_file_path(temp.path(), "github.com").unwrap();
        fs::write(token_path, b"broken").unwrap();

        assert!(load_token_at(temp.path(), "github.com").is_err());
    }

    #[test]
    fn missing_host_returns_none() {
        let temp = TempDir::new().unwrap();
        assert_eq!(load_token_at(temp.path(), "github.com").unwrap(), None);
        assert!(!delete_token_at(temp.path(), "github.com").unwrap());
    }
}
