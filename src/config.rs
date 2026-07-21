use std::path::PathBuf;

use crate::constants;

#[derive(Clone, Debug)]
pub struct Config {
    pub bind_addr: String,
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
            bind_addr: env_str(
                constants::ENV_BIND_ADDR,
                constants::DEFAULT_BIND_ADDR,
            ),
            nsjail_path: env_str(
                constants::ENV_NSJAIL_PATH,
                constants::DEFAULT_NSJAIL_BIN,
            ),
            sandbox_python: env_str(
                constants::ENV_SANDBOX_PYTHON,
                constants::DEFAULT_PYTHON,
            ),
            system_python: env_str(
                constants::ENV_SYSTEM_PYTHON,
                constants::DEFAULT_PYTHON,
            ),
            wrapper_path: {
                let from_env = std::env::var(constants::ENV_WRAPPER_PATH).ok();
                match from_env {
                    Some(p) => PathBuf::from(p),
                    None => {
                        std::env::current_exe()
                            .ok()
                            .and_then(|p| {
                                p.parent()
                                    .map(|d| d.join(constants::DEFAULT_WRAPPER_PATH))
                            })
                            .unwrap_or_else(|| {
                                PathBuf::from(constants::DEFAULT_WRAPPER_PATH)
                            })
                    }
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
