use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::constants;

#[derive(Error, Debug, PartialEq)]
pub enum ConfigError {
    #[error("{key}: expected a boolean (1/true/yes/on or 0/false/no/off), got {value:?}")]
    Bool { key: String, value: String },
    #[error("{key}: expected a positive integer, got {value:?}")]
    Number { key: String, value: String },
    #[error("{key}: expected host:port, got {value:?}")]
    BindAddr { key: String, value: String },
    #[error("{key}: workspace roots must be absolute paths, got {value:?}")]
    RelativeRoot { key: String, value: String },
    #[error("{key}: at least one workspace root is required")]
    NoRoots { key: String },
    #[error("wrapper script not found at {0}; set {1}")]
    MissingWrapper(String, String),
    #[error("{required} demands a sandbox but {enabled} turns it off")]
    Contradiction { required: String, enabled: String },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub bind_addr: String,
    pub sandbox_enabled: bool,
    /// Fail session creation instead of silently downgrading when nsjail is unavailable.
    pub sandbox_required: bool,
    pub nsjail_path: String,
    pub sandbox_python: String,
    pub system_python: String,
    pub wrapper_path: PathBuf,
    pub sandbox_time_limit_secs: u64,
    pub sandbox_memory_limit_bytes: u64,
    pub exec_timeout_secs: u64,
    pub max_sessions: usize,
    /// Directories the file tools may touch. Everything else is refused.
    pub workspace_roots: Vec<PathBuf>,
    pub worker_memory_limit_bytes: u64,
    pub worker_cpu_limit_secs: u64,
    pub worker_file_limit_bytes: u64,
    pub worker_process_limit: u64,
    pub log_level: String,
}

impl Config {
    /// Build from the environment, rejecting anything malformed.
    ///
    /// Bad configuration fails at boot rather than at the first tool call: a server that starts
    /// with a silently-defaulted bind address or an unreachable wrapper is worse than one that
    /// refuses to start.
    pub fn from_env() -> Result<Self, ConfigError> {
        let config = Self {
            bind_addr: env_str(constants::ENV_BIND_ADDR, constants::DEFAULT_BIND_ADDR),
            sandbox_enabled: env_bool(constants::ENV_SANDBOX_ENABLED, true)?,
            sandbox_required: env_bool(constants::ENV_SANDBOX_REQUIRED, false)?,
            nsjail_path: env_str(constants::ENV_NSJAIL_PATH, constants::DEFAULT_NSJAIL_BIN),
            sandbox_python: env_str(constants::ENV_SANDBOX_PYTHON, constants::DEFAULT_PYTHON),
            system_python: env_str(constants::ENV_SYSTEM_PYTHON, constants::DEFAULT_PYTHON),
            wrapper_path: default_wrapper_path(),
            sandbox_time_limit_secs: env_u64(
                constants::ENV_SANDBOX_TIME_LIMIT,
                constants::SANDBOX_TIME_LIMIT_SECS,
            )?,
            sandbox_memory_limit_bytes: env_u64(
                constants::ENV_SANDBOX_MEMORY_LIMIT,
                constants::SANDBOX_MEMORY_LIMIT_BYTES,
            )?,
            exec_timeout_secs: env_u64(constants::ENV_EXEC_TIMEOUT, constants::EXEC_TIMEOUT_SECS)?,
            max_sessions: env_u64(constants::ENV_MAX_SESSIONS, constants::MAX_SESSIONS as u64)?
                as usize,
            workspace_roots: env_roots(constants::ENV_WORKSPACE_ROOTS)?,
            worker_memory_limit_bytes: constants::WORKER_MEMORY_LIMIT_BYTES,
            worker_cpu_limit_secs: constants::WORKER_CPU_LIMIT_SECS,
            worker_file_limit_bytes: constants::WORKER_FILE_LIMIT_BYTES,
            worker_process_limit: constants::WORKER_PROCESS_LIMIT,
            log_level: env_str(constants::ENV_LOG_LEVEL, constants::DEFAULT_LOG_LEVEL),
        };
        config.validate()?;
        Ok(config)
    }

    pub fn socket_addr(&self) -> Result<SocketAddr, ConfigError> {
        self.bind_addr.parse().map_err(|_| ConfigError::BindAddr {
            key: constants::ENV_BIND_ADDR.into(),
            value: self.bind_addr.clone(),
        })
    }

    fn validate(&self) -> Result<(), ConfigError> {
        self.socket_addr()?;
        if self.sandbox_required && !self.sandbox_enabled {
            // Asking for a mandatory sandbox while disabling it can only be a mistake, and the
            // two settings would otherwise resolve quietly in favour of "no sandbox".
            return Err(ConfigError::Contradiction {
                required: constants::ENV_SANDBOX_REQUIRED.into(),
                enabled: constants::ENV_SANDBOX_ENABLED.into(),
            });
        }
        if !self.wrapper_path.exists() {
            return Err(ConfigError::MissingWrapper(
                self.wrapper_path.display().to_string(),
                constants::ENV_WRAPPER_PATH.into(),
            ));
        }
        Ok(())
    }

    /// One-line summary of the settings that change behaviour, for the startup log.
    pub fn summary(&self) -> String {
        let roots: Vec<String> = self
            .workspace_roots
            .iter()
            .map(|p| p.display().to_string())
            .collect();
        format!(
            "sandbox_enabled={} sandbox_required={} exec_timeout={}s max_sessions={} workspace_roots=[{}]",
            self.sandbox_enabled,
            self.sandbox_required,
            self.exec_timeout_secs,
            self.max_sessions,
            roots.join(", "),
        )
    }
}

fn default_wrapper_path() -> PathBuf {
    if let Ok(p) = std::env::var(constants::ENV_WRAPPER_PATH) {
        return PathBuf::from(p);
    }
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join(constants::DEFAULT_WRAPPER_PATH)))
        .unwrap_or_else(|| PathBuf::from(constants::DEFAULT_WRAPPER_PATH))
}

fn env_str(key: &str, default: &str) -> String {
    std::env::var(key)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| default.to_string())
}

fn env_u64(key: &str, default: u64) -> Result<u64, ConfigError> {
    let Some(raw) = std::env::var(key).ok().filter(|v| !v.trim().is_empty()) else {
        return Ok(default);
    };
    match raw.trim().parse::<u64>() {
        Ok(v) if v > 0 => Ok(v),
        _ => Err(ConfigError::Number {
            key: key.into(),
            value: raw,
        }),
    }
}

fn env_bool(key: &str, default: bool) -> Result<bool, ConfigError> {
    let Some(raw) = std::env::var(key).ok().filter(|v| !v.trim().is_empty()) else {
        return Ok(default);
    };
    match raw.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => Err(ConfigError::Bool {
            key: key.into(),
            value: raw,
        }),
    }
}

fn env_roots(key: &str) -> Result<Vec<PathBuf>, ConfigError> {
    let raw = env_str(key, constants::DEFAULT_WORKSPACE_ROOT);
    let mut roots = Vec::new();
    for part in raw.split(constants::WORKSPACE_ROOTS_SEPARATOR) {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let path = PathBuf::from(part);
        if !is_absolute(&path) {
            return Err(ConfigError::RelativeRoot {
                key: key.into(),
                value: part.to_string(),
            });
        }
        roots.push(path);
    }
    if roots.is_empty() {
        return Err(ConfigError::NoRoots { key: key.into() });
    }
    Ok(roots)
}

/// `Path::is_absolute` is false for `/workspace` on Windows (no drive prefix), which would reject
/// the Linux deployment's own default when the config tests run on a dev machine. Rooted is the
/// property that actually matters: it is what makes containment checkable.
fn is_absolute(path: &Path) -> bool {
    path.has_root()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    /// Env vars are process-global; these tests set and clear disjoint keys, and each asserts
    /// only on the key it owns.
    fn clean(key: &str) {
        env::remove_var(key);
    }

    fn base_config() -> Config {
        Config {
            bind_addr: constants::DEFAULT_BIND_ADDR.into(),
            sandbox_enabled: false,
            sandbox_required: false,
            nsjail_path: constants::DEFAULT_NSJAIL_BIN.into(),
            sandbox_python: constants::DEFAULT_PYTHON.into(),
            system_python: constants::DEFAULT_PYTHON.into(),
            wrapper_path: PathBuf::from(constants::DEFAULT_WRAPPER_PATH),
            sandbox_time_limit_secs: constants::SANDBOX_TIME_LIMIT_SECS,
            sandbox_memory_limit_bytes: constants::SANDBOX_MEMORY_LIMIT_BYTES,
            exec_timeout_secs: constants::EXEC_TIMEOUT_SECS,
            max_sessions: constants::MAX_SESSIONS,
            workspace_roots: vec![PathBuf::from(constants::DEFAULT_WORKSPACE_ROOT)],
            worker_memory_limit_bytes: constants::WORKER_MEMORY_LIMIT_BYTES,
            worker_cpu_limit_secs: constants::WORKER_CPU_LIMIT_SECS,
            worker_file_limit_bytes: constants::WORKER_FILE_LIMIT_BYTES,
            worker_process_limit: constants::WORKER_PROCESS_LIMIT,
            log_level: constants::DEFAULT_LOG_LEVEL.into(),
        }
    }

    #[test]
    fn env_bool_accepts_documented_true_values() {
        for v in ["1", "true", "TRUE", "yes", "ON"] {
            env::set_var("__TEST_BOOL_T", v);
            assert_eq!(env_bool("__TEST_BOOL_T", false), Ok(true), "value {v}");
        }
        clean("__TEST_BOOL_T");
        assert_eq!(env_bool("__TEST_BOOL_T", true), Ok(true));
    }

    #[test]
    fn env_bool_accepts_documented_false_values() {
        for v in ["0", "false", "no", "OFF"] {
            env::set_var("__TEST_BOOL_F", v);
            assert_eq!(env_bool("__TEST_BOOL_F", true), Ok(false), "value {v}");
        }
        clean("__TEST_BOOL_F");
    }

    /// A typo used to read as `false`, so `LIBERADO_SANDBOX_ENABLED=ture` silently disabled the
    /// sandbox. Unparseable input is now a boot failure.
    #[test]
    fn env_bool_rejects_garbage_instead_of_defaulting() {
        env::set_var("__TEST_BOOL_X", "ture");
        assert!(matches!(
            env_bool("__TEST_BOOL_X", true),
            Err(ConfigError::Bool { .. })
        ));
        clean("__TEST_BOOL_X");
    }

    #[test]
    fn env_u64_rejects_garbage_and_zero() {
        env::set_var("__TEST_U64_X", "not_a_number");
        assert!(env_u64("__TEST_U64_X", 100).is_err());
        env::set_var("__TEST_U64_X", "0");
        assert!(
            env_u64("__TEST_U64_X", 100).is_err(),
            "zero timeouts/limits disable the protection they configure"
        );
        env::set_var("__TEST_U64_X", "999");
        assert_eq!(env_u64("__TEST_U64_X", 1), Ok(999));
        clean("__TEST_U64_X");
        assert_eq!(env_u64("__TEST_U64_X", 42), Ok(42));
    }

    #[test]
    fn env_str_falls_back_on_empty() {
        clean("__TEST_STR");
        assert_eq!(env_str("__TEST_STR", "fallback"), "fallback");
        env::set_var("__TEST_STR", "   ");
        assert_eq!(env_str("__TEST_STR", "fallback"), "fallback");
        env::set_var("__TEST_STR", "custom");
        assert_eq!(env_str("__TEST_STR", "fallback"), "custom");
        clean("__TEST_STR");
    }

    #[test]
    fn workspace_roots_default_and_split() {
        clean("__TEST_ROOTS");
        assert_eq!(
            env_roots("__TEST_ROOTS").unwrap(),
            vec![PathBuf::from(constants::DEFAULT_WORKSPACE_ROOT)]
        );
        env::set_var("__TEST_ROOTS", "/workspace:/data");
        assert_eq!(
            env_roots("__TEST_ROOTS").unwrap(),
            vec![PathBuf::from("/workspace"), PathBuf::from("/data")]
        );
        clean("__TEST_ROOTS");
    }

    /// A relative root cannot be checked against a canonicalised path, so it would silently
    /// allow nothing (or, worse, everything under the process CWD).
    #[test]
    fn workspace_roots_reject_relative_paths() {
        env::set_var("__TEST_ROOTS_REL", "workspace");
        assert!(matches!(
            env_roots("__TEST_ROOTS_REL"),
            Err(ConfigError::RelativeRoot { .. })
        ));
        clean("__TEST_ROOTS_REL");
    }

    #[test]
    fn bind_addr_must_parse() {
        let mut config = base_config();
        config.bind_addr = "not-an-address".into();
        assert!(matches!(
            config.socket_addr(),
            Err(ConfigError::BindAddr { .. })
        ));
        config.bind_addr = "0.0.0.0:8000".into();
        assert!(config.socket_addr().is_ok());
    }

    /// Every session spawns the wrapper. Missing at boot means every call fails later, so the
    /// server refuses to start instead.
    #[test]
    fn validate_rejects_missing_wrapper() {
        let mut config = base_config();
        config.wrapper_path = PathBuf::from("/nonexistent/wrapper.py");
        assert!(matches!(
            config.validate(),
            Err(ConfigError::MissingWrapper(..))
        ));
    }

    /// "Required but disabled" would otherwise resolve silently in favour of no sandbox — the
    /// opposite of what the operator asked for.
    #[test]
    fn validate_rejects_required_but_disabled_sandbox() {
        let mut config = base_config();
        config.wrapper_path = std::env::current_dir().unwrap().join("sandbox/wrapper.py");
        config.sandbox_required = true;
        config.sandbox_enabled = false;
        assert!(matches!(
            config.validate(),
            Err(ConfigError::Contradiction { .. })
        ));
    }

    #[test]
    fn validate_accepts_a_real_wrapper() {
        let mut config = base_config();
        config.wrapper_path = std::env::current_dir().unwrap().join("sandbox/wrapper.py");
        assert_eq!(config.validate(), Ok(()));
    }

    #[test]
    fn summary_names_the_security_relevant_settings() {
        let summary = base_config().summary();
        assert!(summary.contains("sandbox_enabled"));
        assert!(summary.contains("exec_timeout"));
        assert!(summary.contains("workspace_roots"));
    }
}
