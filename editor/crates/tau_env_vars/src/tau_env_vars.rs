pub use env_var::{EnvVar, bool_env_var, env_var};
use std::sync::LazyLock;

/// Whether Tau is running in stateless mode.
/// When true, Tau will use in-memory databases instead of persistent storage.
pub static TAU_STATELESS: LazyLock<bool> = bool_env_var!("TAU_STATELESS");
