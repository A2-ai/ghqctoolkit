#[cfg(test)]
use mockall::automock;

// Trait for environment variable access (mockable in tests)
#[cfg_attr(test, automock)]
pub trait EnvProvider {
    fn var(&self, key: &str) -> Result<String, std::env::VarError>;
    fn set_var(&self, key: &str, value: &str);
}

// Default implementation that uses std::env
#[derive(Default)]
pub struct StdEnvProvider;

impl EnvProvider for StdEnvProvider {
    fn var(&self, key: &str) -> Result<String, std::env::VarError> {
        std::env::var(key)
    }

    fn set_var(&self, key: &str, value: &str) {
        // SAFETY: We control when this is called and ensure it's not during
        // concurrent access to environment variables
        unsafe { std::env::set_var(key, value) }
    }
}
