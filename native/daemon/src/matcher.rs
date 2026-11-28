//! The selected processes.
//!
//! An entry is a file name (`chrome.exe`), a full path (an entry with a separator) or a regular expression (an entry that starts with `re:`). A file name must be the same as the file name of the executable. A full path must be the same as the whole path. A regular expression must find a match in the file name of the executable. The anchors `^` and `$` are possible. Case does not matter, as for Windows file names.

use anyhow::{Context as _, Result};
use regex::{Regex, RegexBuilder};

const REGEX_PREFIX: &str = "re:";

#[derive(Debug, Clone, Default)]
pub struct Matcher {
    names: Vec<String>,
    paths: Vec<String>,
    patterns: Vec<Regex>,
}

fn normalize(path: &str) -> String {
    path.replace('/', "\\").to_lowercase()
}

impl Matcher {
    /// Fails if a regular expression is not valid.
    pub fn new<S: AsRef<str>>(entries: &[S]) -> Result<Self> {
        let mut matcher = Self::default();
        for entry in entries {
            let entry = entry.as_ref().trim();
            if entry.is_empty() {
                continue;
            }
            if let Some(pattern) = entry.strip_prefix(REGEX_PREFIX) {
                let regex = RegexBuilder::new(pattern)
                    .case_insensitive(true)
                    .build()
                    .with_context(|| format!("The regular expression {pattern:?} is not valid"))?;
                matcher.patterns.push(regex);
            } else if entry.contains(['\\', '/']) {
                matcher.paths.push(normalize(entry));
            } else {
                matcher.names.push(entry.to_lowercase());
            }
        }
        Ok(matcher)
    }

    pub fn is_empty(&self) -> bool {
        self.names.is_empty() && self.paths.is_empty() && self.patterns.is_empty()
    }

    /// True if the executable at `image` (a full path) is selected.
    pub fn matches(&self, image: &str) -> bool {
        let image = normalize(image);
        let file = image.rsplit('\\').next().unwrap_or(&image);
        self.names.iter().any(|name| name == file)
            || self.paths.contains(&image)
            || self.patterns.iter().any(|pattern| pattern.is_match(file))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_match_the_file_name_ignoring_case() {
        let m = Matcher::new(&["Chrome.exe"]).unwrap();
        assert!(m.matches(r"C:\Program Files\Google\Chrome\application\chrome.exe"));
        assert!(!m.matches(r"C:\tools\chrome.exe.bak"));
        assert!(!m.matches(r"C:\tools\notchrome.exe"));
    }

    #[test]
    fn paths_match_the_whole_path() {
        let m = Matcher::new(&["C:/Tools/App.exe"]).unwrap();
        assert!(m.matches(r"c:\tools\app.exe"));
        assert!(!m.matches(r"d:\tools\app.exe"));
    }

    #[test]
    fn regex_entries_search_the_file_name_ignoring_case() {
        let m = Matcher::new(&[r"re:^chrome(_proxy)?\.exe$"]).unwrap();
        assert!(m.matches(r"C:\x\Chrome.exe"));
        assert!(m.matches(r"C:\x\chrome_proxy.exe"));
        assert!(!m.matches(r"C:\x\chrome.exe.bak"));
        let m = Matcher::new(&["re:help"]).unwrap();
        assert!(m.matches(r"C:\x\My Helper.exe"));
        assert!(!m.matches(r"C:\help\other.exe"));
    }

    #[test]
    fn a_bad_regex_is_an_error() {
        assert!(Matcher::new(&[r"re:(\.exe"]).is_err());
    }

    #[test]
    fn non_ascii_names_work() {
        assert!(
            Matcher::new(&["哔哩哔哩.exe"])
                .unwrap()
                .matches(r"C:\x\哔哩哔哩.exe")
        );
    }

    #[test]
    fn empty_entries_are_ignored() {
        assert!(Matcher::new(&["", "  "]).unwrap().is_empty());
    }
}
