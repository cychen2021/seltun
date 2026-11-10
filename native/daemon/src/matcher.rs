//! Which processes are selected.
//!
//! An entry is either a file name (`chrome.exe`), matched against the executable's file name, or a full path (anything with a separator), matched against the whole path. Both are case-insensitive, as Windows file names are.

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Matcher {
    names: Vec<String>,
    paths: Vec<String>,
}

fn normalize(path: &str) -> String {
    path.replace('/', "\\").to_lowercase()
}

impl Matcher {
    pub fn new<S: AsRef<str>>(entries: &[S]) -> Self {
        let mut matcher = Self::default();
        for entry in entries {
            let entry = entry.as_ref().trim();
            if entry.is_empty() {
                continue;
            }
            if entry.contains(['\\', '/']) {
                matcher.paths.push(normalize(entry));
            } else {
                matcher.names.push(entry.to_lowercase());
            }
        }
        matcher
    }

    pub fn is_empty(&self) -> bool {
        self.names.is_empty() && self.paths.is_empty()
    }

    /// Whether the executable at `image` (a full path) is selected.
    pub fn matches(&self, image: &str) -> bool {
        let image = normalize(image);
        let file = image.rsplit('\\').next().unwrap_or(&image);
        self.names.iter().any(|name| name == file) || self.paths.contains(&image)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_match_the_file_name_ignoring_case() {
        let m = Matcher::new(&["Chrome.exe"]);
        assert!(m.matches(r"C:\Program Files\Google\Chrome\application\chrome.exe"));
        assert!(!m.matches(r"C:\tools\chrome.exe.bak"));
        assert!(!m.matches(r"C:\tools\notchrome.exe"));
    }

    #[test]
    fn paths_match_the_whole_path() {
        let m = Matcher::new(&["C:/Tools/App.exe"]);
        assert!(m.matches(r"c:\tools\app.exe"));
        assert!(!m.matches(r"d:\tools\app.exe"));
    }

    #[test]
    fn non_ascii_names_work() {
        assert!(Matcher::new(&["哔哩哔哩.exe"]).matches(r"C:\x\哔哩哔哩.exe"));
    }

    #[test]
    fn empty_entries_are_ignored() {
        assert!(Matcher::new(&["", "  "]).is_empty());
    }
}
