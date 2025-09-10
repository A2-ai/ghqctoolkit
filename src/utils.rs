#[cfg(test)]
use mockall::automock;

// Trait for environment variable access (mockable in tests)
#[cfg_attr(test, automock)]
pub trait EnvProvider {
    fn var(&self, key: &str) -> Result<String, std::env::VarError>;
}

// Default implementation that uses std::env
#[derive(Default)]
pub struct StdEnvProvider;

impl EnvProvider for StdEnvProvider {
    fn var(&self, key: &str) -> Result<String, std::env::VarError> {
        std::env::var(key)
    }
}
