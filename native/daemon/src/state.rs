//! `state.json`: the saved on/off switch. The Python CLI also uses this file.
//!
//! The CLI writes the file (`seltun switch on` and `seltun switch off`) and tells the daemon to reload. The daemon only reads the file. Thus the daemon comes back in the state that the user selected when it starts again.

use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Default, Clone, PartialEq, Eq)]
pub struct State {
    /// If true, the daemon steers the selected processes into the TUN adapter. If the file is missing, the value is false.
    #[serde(default)]
    pub enabled: bool,
}

impl State {
    pub fn load(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            // Windows editors can add a byte order mark. JSON does not accept it.
            Ok(text) => serde_json::from_str(text.trim_start_matches('\u{feff}'))
                .with_context(|| format!("Cannot parse {}", path.display())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error).with_context(|| format!("Cannot read {}", path.display())),
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
    fn accepts_a_byte_order_mark() {
        let path = std::env::temp_dir().join("seltun-state-bom.json");
        std::fs::write(&path, "\u{feff}{\"enabled\": true}").unwrap();
        assert!(State::load(&path).unwrap().enabled);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn reads_enabled_and_ignores_unknown_keys() {
        let path = std::env::temp_dir().join("seltun-state-test.json");
        std::fs::write(&path, r#"{"enabled": true, "future": 1}"#).unwrap();
        assert!(State::load(&path).unwrap().enabled);
        std::fs::remove_file(path).unwrap();
    }
}
