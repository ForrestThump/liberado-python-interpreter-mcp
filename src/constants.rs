pub const SERVER_NAME: &str = "liberado-python-interpreter-mcp";
pub const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

pub const DEFAULT_NSJAIL_BIN: &str = "nsjail";
pub const DEFAULT_PYTHON: &str = "python3";
pub const DEFAULT_WRAPPER_PATH: &str = "sandbox/wrapper.py";
pub const DEFAULT_BIND_ADDR: &str = "0.0.0.0:8000";
pub const DEFAULT_LOG_LEVEL: &str = "info";
pub const DEFAULT_WORKSPACE_ROOT: &str = "/workspace";

pub const SANDBOX_TIME_LIMIT_SECS: u64 = 300;
pub const SANDBOX_MEMORY_LIMIT_BYTES: u64 = 512 * 1024 * 1024;
pub const SANDBOX_WORK_DIR: &str = "/work";
pub const SANDBOX_CHROOT: &str = "/";

/// Wall-clock ceiling on a single `execute_python` call. Without it a `while True: pass` blocks
/// the calling task forever; with the old global lock that wedged the entire server.
pub const EXEC_TIMEOUT_SECS: u64 = 120;

/// Worker rlimits, applied by the wrapper to itself. These bound one session; the container's
/// own memory/pids caps bound the server as a whole.
pub const WORKER_MEMORY_LIMIT_BYTES: u64 = 512 * 1024 * 1024;
pub const WORKER_CPU_LIMIT_SECS: u64 = 300;
pub const WORKER_FILE_LIMIT_BYTES: u64 = 256 * 1024 * 1024;
pub const WORKER_PROCESS_LIMIT: u64 = 64;

pub const SESSION_IDLE_SECONDS: u64 = 1800;
pub const MAX_SESSIONS: usize = 32;
/// How often the background reaper sweeps idle sessions. Cleanup used to run only inside
/// `execute_python`, so an abandoned session held its subprocess until the next unrelated call.
pub const CLEANUP_INTERVAL_SECS: u64 = 60;

pub const MAX_OUTPUT_BYTES: usize = 50_000;

pub const ENV_BIND_ADDR: &str = "BIND_ADDR";
pub const ENV_NSJAIL_PATH: &str = "NSJAIL_PATH";
pub const ENV_SANDBOX_PYTHON: &str = "SANDBOX_PYTHON";
pub const ENV_SYSTEM_PYTHON: &str = "SYSTEM_PYTHON";
pub const ENV_WRAPPER_PATH: &str = "LIBERADO_WRAPPER_PATH";
pub const ENV_SANDBOX_ENABLED: &str = "LIBERADO_SANDBOX_ENABLED";
pub const ENV_SANDBOX_REQUIRED: &str = "LIBERADO_SANDBOX_REQUIRED";
pub const ENV_LOG_LEVEL: &str = "RUST_LOG";
pub const ENV_EXEC_TIMEOUT: &str = "LIBERADO_EXEC_TIMEOUT";
pub const ENV_MAX_SESSIONS: &str = "LIBERADO_MAX_SESSIONS";
pub const ENV_WORKSPACE_ROOTS: &str = "LIBERADO_WORKSPACE_ROOTS";
pub const ENV_SANDBOX_TIME_LIMIT: &str = "LIBERADO_SANDBOX_TIME_LIMIT";
pub const ENV_SANDBOX_MEMORY_LIMIT: &str = "LIBERADO_SANDBOX_MEMORY_LIMIT";

/// Passed to the wrapper process, not read by the server.
pub const ENV_PACKAGES_DIR: &str = "LIBERADO_PACKAGES_DIR";
pub const ENV_LIMIT_MEMORY: &str = "LIBERADO_LIMIT_MEMORY_BYTES";
pub const ENV_LIMIT_CPU: &str = "LIBERADO_LIMIT_CPU_SECONDS";
pub const ENV_LIMIT_FILE: &str = "LIBERADO_LIMIT_FILE_BYTES";
pub const ENV_LIMIT_PROCS: &str = "LIBERADO_LIMIT_PROCESSES";

pub const WORKSPACE_ROOTS_SEPARATOR: char = ':';

pub const PROTO_CMD: &str = "cmd";
pub const PROTO_CODE: &str = "code";
pub const PROTO_EXEC: &str = "exec";
pub const PROTO_STDOUT: &str = "stdout";
pub const PROTO_STDERR: &str = "stderr";
pub const PROTO_MORE_INPUT: &str = "more_input_needed";

pub const KEY_SESSION_ID: &str = "session_id";
pub const KEY_CREATED: &str = "created";
pub const KEY_SANDBOXED: &str = "sandboxed";
pub const KEY_TRUNCATED_STDOUT: &str = "truncated_stdout";
pub const KEY_TRUNCATED_STDERR: &str = "truncated_stderr";
pub const KEY_RESET: &str = "reset";
pub const KEY_SECONDS_IDLE: &str = "seconds_idle";
pub const KEY_SECONDS_ALIVE: &str = "seconds_alive";
pub const KEY_PATH: &str = "path";
pub const KEY_CONTENT: &str = "content";
pub const KEY_SIZE_BYTES: &str = "size_bytes";
pub const KEY_TRUNCATED: &str = "truncated";
pub const KEY_WRITTEN: &str = "written";
pub const KEY_REPLACED: &str = "replaced";
pub const KEY_RAW: &str = "raw";
pub const KEY_ERROR: &str = "error";
pub const KEY_FIND_NOT_FOUND: &str = "Find string not found in file";

pub const PKGS_DIR: &str = "packages";
pub const PKGS_SYS_PATH_ENTRY: &str = "/work/packages";

pub const PYTHON_INFO_CODE: &str = "import sys,json;print(json.dumps({'version':sys.version,'executable':sys.executable,'platform':sys.platform,'prefix':sys.prefix}))";

pub const PYTHON_UNBUFFERED: &str = "-u";
pub const PYTHON_C_ARG: &str = "-c";

pub const PIP_INSTALL: &str = "install";
pub const PIP_LIST: &str = "list";
pub const PIP_FORMAT_ARG: &str = "--format=json";
pub const PIP_MODULE: &str = "-m";
pub const PIP_CMD: &str = "pip";
pub const PIP_NO_INPUT: &str = "--no-input";
pub const PIP_TARGET_ARG: &str = "--target";
pub const PIP_END_OF_OPTIONS: &str = "--";

pub const NSJAIL_MODE_ARG: &str = "--mode";
pub const NSJAIL_MODE_EXEC: &str = "exec";
pub const NSJAIL_CHROOT_ARG: &str = "--chroot";
pub const NSJAIL_BINDMOUNT_ARG: &str = "--bindmount";
pub const NSJAIL_CWD_ARG: &str = "--cwd";
pub const NSJAIL_DISABLE_PROC: &str = "--disable_proc";
pub const NSJAIL_IFACE_NO_LO: &str = "--iface_no_lo";
pub const NSJAIL_REALLY_QUIET: &str = "--really_quiet";
pub const NSJAIL_TIME_LIMIT_ARG: &str = "--time_limit";
pub const NSJAIL_MEMORY_LIMIT_ARG: &str = "--cgroup_mem_max";
pub const NSJAIL_CMD_SEP: &str = "--";
