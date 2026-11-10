//! `state.json`: the persisted on/off switch, shared with the Python CLI.
//!
//! The CLI writes it (`seltun on` / `seltun off`) and tells the daemon to reload; the daemon only reads it, so a restart comes back in the state the user left.

use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Default, Clone, PartialEq, Eq)]
pub struct State {
    /// Whether the selected processes are steered into the TUN adapter. Off when there is no file.
    #[serde(default)]
    pub enabled: bool,
}

impl State {
    pub fn load(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(text) => {
                serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error).with_context(|| format!("reading {}", path.display())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_is_off() {
        let dir = std::env::temp_dir().join("seltun-state-missing");
        assert!(!State::load(&dir.join("state.json")).unwrap().enabled);
    }

    #[test]
    fn reads_enabled_and_ignores_unknown_keys() {
        let path = std::env::temp_dir().join("seltun-state-test.json");
        std::fs::write(&path, r#"{"enabled": true, "future": 1}"#).unwrap();
        assert!(State::load(&path).unwrap().enabled);
        std::fs::remove_file(path).unwrap();
    }
}
