use std::path::PathBuf;

use crate::constants;

#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub bind_addr: String,
    pub sandbox_enabled: bool,
    pub nsjail_path: String,
    pub sandbox_python: String,
    pub system_python: String,
    pub wrapper_path: PathBuf,
    pub sandbox_time_limit_secs: u64,
    pub sandbox_memory_limit_bytes: u64,
    pub log_level: String,
}

impl Config {
    pub fn from_env() -> Self {
        Self {
            bind_addr: env_str(constants::ENV_BIND_ADDR, constants::DEFAULT_BIND_ADDR),
            sandbox_enabled: env_bool(constants::ENV_SANDBOX_ENABLED, true),
            nsjail_path: env_str(constants::ENV_NSJAIL_PATH, constants::DEFAULT_NSJAIL_BIN),
            sandbox_python: env_str(constants::ENV_SANDBOX_PYTHON, constants::DEFAULT_PYTHON),
            system_python: env_str(constants::ENV_SYSTEM_PYTHON, constants::DEFAULT_PYTHON),
            wrapper_path: {
                let from_env = std::env::var(constants::ENV_WRAPPER_PATH).ok();
                match from_env {
                    Some(p) => PathBuf::from(p),
                    None => std::env::current_exe()
                        .ok()
                        .and_then(|p| p.parent().map(|d| d.join(constants::DEFAULT_WRAPPER_PATH)))
                        .unwrap_or_else(|| PathBuf::from(constants::DEFAULT_WRAPPER_PATH)),
                }
            },
            sandbox_time_limit_secs: env_u64(
                "LIBERADO_SANDBOX_TIME_LIMIT",
                constants::SANDBOX_TIME_LIMIT_SECS,
            ),
            sandbox_memory_limit_bytes: env_u64(
                "LIBERADO_SANDBOX_MEMORY_LIMIT",
                constants::SANDBOX_MEMORY_LIMIT_BYTES,
            ),
            log_level: env_str(constants::ENV_LOG_LEVEL, constants::DEFAULT_LOG_LEVEL),
        }
    }
}

fn env_str(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

fn env_u64(key: &str, default: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn env_bool(key: &str, default: bool) -> bool {
    std::env::var(key)
        .ok()
        .and_then(|v| {
            let v = v.trim().to_lowercase();
            if v.is_empty() {
                None
            } else if matches!(v.as_str(), "1" | "true" | "yes" | "on") {
                Some(true)
            } else {
                Some(false)
            }
        })
        .unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    fn clean(key: &str) {
        env::remove_var(key);
    }

    #[test]
    fn test_env_bool_true_values() {
        clean("__TEST_BOOL");
        assert!(env_bool("__TEST_BOOL", true));
        env::set_var("__TEST_BOOL", "1");
        assert!(env_bool("__TEST_BOOL", false));
        env::set_var("__TEST_BOOL", "true");
        assert!(env_bool("__TEST_BOOL", false));
        env::set_var("__TEST_BOOL", "yes");
        assert!(env_bool("__TEST_BOOL", false));
        env::set_var("__TEST_BOOL", "ON");
        assert!(env_bool("__TEST_BOOL", false));
        clean("__TEST_BOOL");
    }

    #[test]
    fn test_env_bool_false_values() {
        clean("__TEST_BOOL2");
        env::set_var("__TEST_BOOL2", "0");
        assert!(!env_bool("__TEST_BOOL2", true));
        env::set_var("__TEST_BOOL2", "false");
        assert!(!env_bool("__TEST_BOOL2", true));
        env::set_var("__TEST_BOOL2", "no");
        assert!(!env_bool("__TEST_BOOL2", true));
        env::set_var("__TEST_BOOL2", "off");
        assert!(!env_bool("__TEST_BOOL2", true));
        clean("__TEST_BOOL2");
    }

    #[test]
    fn test_env_u64_parsing() {
        clean("__TEST_U64");
        assert_eq!(env_u64("__TEST_U64", 42), 42);
        env::set_var("__TEST_U64", "999");
        assert_eq!(env_u64("__TEST_U64", 0), 999);
        env::set_var("__TEST_U64", "not_a_number");
        assert_eq!(env_u64("__TEST_U64", 100), 100);
        clean("__TEST_U64");
    }

    #[test]
    fn test_env_str_default() {
        clean("__TEST_STR");
        assert_eq!(env_str("__TEST_STR", "fallback"), "fallback");
        env::set_var("__TEST_STR", "custom");
        assert_eq!(env_str("__TEST_STR", "fallback"), "custom");
        clean("__TEST_STR");
    }

    #[test]
    fn test_config_defaults() {
        clean(constants::ENV_BIND_ADDR);
        clean(constants::ENV_SANDBOX_ENABLED);
        clean(constants::ENV_NSJAIL_PATH);
        clean("LIBERADO_SANDBOX_TIME_LIMIT");
        clean("LIBERADO_SANDBOX_MEMORY_LIMIT");

        let config = Config::from_env();
        assert_eq!(config.bind_addr, constants::DEFAULT_BIND_ADDR);
        assert!(config.sandbox_enabled);
        assert_eq!(config.nsjail_path, constants::DEFAULT_NSJAIL_BIN);
        assert_eq!(
            config.sandbox_time_limit_secs,
            constants::SANDBOX_TIME_LIMIT_SECS
        );
        assert_eq!(
            config.sandbox_memory_limit_bytes,
            constants::SANDBOX_MEMORY_LIMIT_BYTES
        );
    }

    #[test]
    fn test_config_sandbox_disable() {
        clean(constants::ENV_SANDBOX_ENABLED);
        env::set_var(constants::ENV_SANDBOX_ENABLED, "0");
        let val = env_bool(constants::ENV_SANDBOX_ENABLED, true);
        assert!(
            !val,
            "env_bool should return false when set to '0', got true"
        );
        let config = Config::from_env();
        assert!(
            !config.sandbox_enabled,
            "sandbox_enabled should be false when LIBERADO_SANDBOX_ENABLED=0"
        );
        clean(constants::ENV_SANDBOX_ENABLED);
    }

    #[test]
    fn test_config_override() {
        clean(constants::ENV_BIND_ADDR);
        clean("LIBERADO_SANDBOX_MEMORY_LIMIT");

        env::set_var(constants::ENV_BIND_ADDR, "127.0.0.1:9999");
        env::set_var("LIBERADO_SANDBOX_MEMORY_LIMIT", "268435456");

        let config = Config::from_env();
        assert_eq!(config.bind_addr, "127.0.0.1:9999");
        assert_eq!(config.sandbox_memory_limit_bytes, 268435456);

        clean(constants::ENV_BIND_ADDR);
        clean("LIBERADO_SANDBOX_MEMORY_LIMIT");
    }
}
