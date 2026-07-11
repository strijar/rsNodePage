//! Scans `pages_dir` and `files_dir` and builds an in-memory index keyed by
//! the truncated hash of the request path — exactly the value RNS request
//! handlers receive as `path_hash`, so lookups are a plain `HashMap::get`.

use std::collections::HashMap;
use std::path::PathBuf;

use rns_crypto::sha::truncated_hash;

/// Where a served path's bytes come from.
#[derive(Debug, Clone)]
pub enum Content {
    /// Read from disk on every request (cheap: pages are small text files).
    File(PathBuf),
    /// Held in memory — used for the auto-generated index page.
    Generated(Vec<u8>),
    /// A `.rhai` script, executed fresh on every request. See `crate::script`.
    Script(PathBuf),
}

#[derive(Debug, Clone)]
pub struct Entry {
    /// The NomadNet request path, e.g. `/page/index.mu` or `/file/manual.pdf`.
    pub request_path: String,
    pub content: Content,
    /// True for `/file/...` entries (sent with a filename in the response
    /// metadata so clients know what to save it as).
    pub is_file: bool,
}

#[derive(Debug, Default)]
pub struct PageIndex {
    entries: HashMap<[u8; 16], Entry>,
}

impl PageIndex {
    pub fn get(&self, path_hash: &[u8; 16]) -> Option<&Entry> {
        self.entries.get(path_hash)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Rebuilds the index by scanning `pages_dir` (served under `/page/`) and
    /// `files_dir` (served under `/file/`). Only top-level files are served —
    /// no recursion into subdirectories in this minimal implementation.
    pub fn scan(pages_dir: &std::path::Path, files_dir: &std::path::Path) -> Self {
        let mut entries = HashMap::new();
        let mut page_names = Vec::new();

        if let Ok(dir) = std::fs::read_dir(pages_dir) {
            for item in dir.flatten() {
                if !item.file_type().map(|t| t.is_file()).unwrap_or(false) {
                    continue;
                }
                let Some(name) = item.file_name().to_str().map(str::to_string) else {
                    continue;
                };

                // `clock.mu.rhai` is served at `/page/clock.mu`, executed on
                // every request instead of read verbatim.
                let (display_name, content) = match name.strip_suffix(".rhai") {
                    Some(stripped) => (stripped.to_string(), Content::Script(item.path())),
                    None => (name.clone(), Content::File(item.path())),
                };

                let request_path = format!("/page/{display_name}");
                let hash = truncated_hash(request_path.as_bytes());
                entries.insert(
                    hash,
                    Entry {
                        request_path: request_path.clone(),
                        content,
                        is_file: false,
                    },
                );
                page_names.push(display_name);
            }
        }

        if let Ok(dir) = std::fs::read_dir(files_dir) {
            for item in dir.flatten() {
                if !item.file_type().map(|t| t.is_file()).unwrap_or(false) {
                    continue;
                }
                let Some(name) = item.file_name().to_str().map(str::to_string) else {
                    continue;
                };
                let request_path = format!("/file/{name}");
                let hash = truncated_hash(request_path.as_bytes());
                entries.insert(
                    hash,
                    Entry {
                        request_path,
                        content: Content::File(item.path()),
                        is_file: true,
                    },
                );
            }
        }

        // Auto-generate /page/index.mu if the operator didn't provide one.
        let index_path = "/page/index.mu";
        let index_hash = truncated_hash(index_path.as_bytes());
        if !entries.contains_key(&index_hash) {
            page_names.sort();
            entries.insert(
                index_hash,
                Entry {
                    request_path: index_path.to_string(),
                    content: Content::Generated(generate_index(&page_names)),
                    is_file: false,
                },
            );
        }

        Self { entries }
    }
}

/// A plain, dependency-free Micron listing page. Link syntax is
/// `` `[Link text`/page/name.mu] ``.
fn generate_index(page_names: &[String]) -> Vec<u8> {
    let mut out = String::new();
    out.push_str(">Pages\n\n");
    if page_names.is_empty() {
        out.push_str("No pages have been published yet.\n");
    } else {
        for name in page_names {
            out.push_str(&format!("`[{name}`/page/{name}]\n"));
        }
    }
    out.into_bytes()
}

/// Resolves the `Content` of an entry into response bytes. Does *not* run
/// scripts — `Content::Script` is handled separately by `crate::script`,
/// since it needs the request's field data and an `Engine` to run against.
pub fn read_content(content: &Content) -> Option<Vec<u8>> {
    match content {
        Content::File(path) => std::fs::read(path).ok(),
        Content::Generated(bytes) => Some(bytes.clone()),
        Content::Script(_) => None,
    }
}
