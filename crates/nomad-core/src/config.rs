//! Configuration for the NomadNetwork page-hosting node.
//!
//! The config file is a plain INI file (same format Reticulum itself uses,
//! parsed via `rns_runtime::config::Config`), with a single `[nomadnetwork]`
//! section. Example:
//!
//! ```ini
//! [nomadnetwork]
//! display_name = My Pages
//! pages_dir = /home/user/nomad/pages
//! files_dir = /home/user/nomad/files
//! announce_at_start = yes
//! announce_interval_minutes = 360
//! rescan_interval_seconds = 30
//! # optional allow-list: only these identity hashes (hex) may fetch pages
//! # allowed_identities = a1b2c3..., d4e5f6...
//! ```

use std::path::{Path, PathBuf};

use rns_runtime::config::Config;

#[derive(Debug, Clone)]
pub struct NomadConfig {
    pub display_name: String,
    pub pages_dir: PathBuf,
    pub files_dir: PathBuf,
    pub identity_path: PathBuf,
    pub announce_at_start: bool,
    pub announce_interval_minutes: u64,
    pub rescan_interval_seconds: u64,
    /// If non-empty, only requests from these identity hashes are served.
    pub allowed_identities: Vec<[u8; 16]>,
}

impl NomadConfig {
    /// Builds a config from an on-disk INI file, or defaults if it doesn't
    /// exist. `config_dir` is where relative paths (identity, default
    /// pages/files dirs) are anchored, unless overridden explicitly.
    pub fn load(config_path: &Path) -> Self {
        let config_dir = config_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));

        let parsed = match Config::from_file(config_path) {
            Ok(c) => Some(c),
            Err(e) => {
                tracing::warn!(
                    path = %config_path.display(),
                    error = %e,
                    "failed to parse config file, using built-in defaults"
                );
                None
            }
        };
        let section = parsed.as_ref().and_then(|c| c.section("nomadnetwork"));
        if parsed.is_some() && section.is_none() {
            tracing::warn!(
                path = %config_path.display(),
                "config file has no [nomadnetwork] section, using built-in defaults"
            );
        }

        let display_name = section
            .and_then(|s| s.get("display_name"))
            .unwrap_or("Anonymous Node")
            .to_string();

        let pages_dir = section
            .and_then(|s| s.get("pages_dir"))
            .map(PathBuf::from)
            .unwrap_or_else(|| config_dir.join("pages"));

        let files_dir = section
            .and_then(|s| s.get("files_dir"))
            .map(PathBuf::from)
            .unwrap_or_else(|| config_dir.join("files"));

        let identity_path = section
            .and_then(|s| s.get("identity_path"))
            .map(PathBuf::from)
            .unwrap_or_else(|| config_dir.join("identity"));

        let announce_at_start = section
            .map(|s| s.get_bool_or("announce_at_start", true))
            .unwrap_or(true);

        let announce_interval_minutes = section
            .and_then(|s| s.get_int("announce_interval_minutes"))
            .filter(|v| *v > 0)
            .map(|v| v as u64)
            .unwrap_or(360);

        let rescan_interval_seconds = section
            .and_then(|s| s.get_int("rescan_interval_seconds"))
            .filter(|v| *v > 0)
            .map(|v| v as u64)
            .unwrap_or(30);

        let allowed_identities = section
            .and_then(|s| s.get_list("allowed_identities"))
            .unwrap_or_default()
            .iter()
            .filter_map(|s| parse_identity_hash(s))
            .collect();

        Self {
            display_name,
            pages_dir,
            files_dir,
            identity_path,
            announce_at_start,
            announce_interval_minutes,
            rescan_interval_seconds,
            allowed_identities,
        }
    }

    /// Example config text, for `--exampleconfig`.
    pub fn example() -> &'static str {
        r#"[nomadnetwork]
# Name shown to peers when they see this node's announce.
display_name = My Pages

# Directories are relative to this config file unless given as absolute paths.
pages_dir = pages
files_dir = files

# Send an announce as soon as the node starts.
announce_at_start = yes
# How often to re-announce, in minutes.
announce_interval_minutes = 360

# How often to rescan pages_dir/files_dir for changes, in seconds.
rescan_interval_seconds = 30

# Optional: restrict serving to specific identity hashes (hex, comma-separated).
# If left empty, the node serves pages to anyone.
# allowed_identities = a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4
"#
    }
}

fn parse_identity_hash(s: &str) -> Option<[u8; 16]> {
    let bytes = hex::decode(s.trim()).ok()?;
    if bytes.len() != 16 {
        return None;
    }
    let mut out = [0u8; 16];
    out.copy_from_slice(&bytes);
    Some(out)
}
