//! The selected processes.
//!
//! An entry is a file name (`chrome.exe`), a full path (an entry with a separator) or a regular expression (an entry that starts with `re:`). A file name must be the same as the file name of the executable. A full path must be the same as the whole path. A regular expression must find a match in the file name of the executable. The syntax is the one of the `fancy-regex` crate. It has anchors (`^`, `$`), lookaround (`(?!x)`, `(?<=x)`) and backreferences. Case does not matter, as for Windows file names.

use anyhow::{Context as _, Result};
use fancy_regex::{Regex, RegexBuilder};

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

    /// True if the executable at `image` (a full path) is selected. Fails if a regular expression fails at run time (for example, it reaches the backtrack limit).
    pub fn matches(&self, image: &str) -> Result<bool> {
        let image = normalize(image);
        let file = image.rsplit('\\').next().unwrap_or(&image);
        if self.names.iter().any(|name| name == file) || self.paths.contains(&image) {
            return Ok(true);
        }
        for pattern in &self.patterns {
            let found = pattern.is_match(file).with_context(|| {
                format!(
                    "The regular expression {:?} failed on {file:?}",
                    pattern.as_str()
                )
            })?;
            if found {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_match_the_file_name_ignoring_case() {
        let m = Matcher::new(&["Chrome.exe"]).unwrap();
        assert!(m.matches(r"C:\Program Files\Google\Chrome\application\chrome.exe").unwrap());
        assert!(!m.matches(r"C:\tools\chrome.exe.bak").unwrap());
        assert!(!m.matches(r"C:\tools\notchrome.exe").unwrap());
    }

    #[test]
    fn paths_match_the_whole_path() {
        let m = Matcher::new(&["C:/Tools/App.exe"]).unwrap();
        assert!(m.matches(r"c:\tools\app.exe").unwrap());
        assert!(!m.matches(r"d:\tools\app.exe").unwrap());
    }

    #[test]
    fn regex_entries_search_the_file_name_ignoring_case() {
        let m = Matcher::new(&[r"re:^chrome(_proxy)?\.exe$"]).unwrap();
        assert!(m.matches(r"C:\x\Chrome.exe").unwrap());
        assert!(m.matches(r"C:\x\chrome_proxy.exe").unwrap());
        assert!(!m.matches(r"C:\x\chrome.exe.bak").unwrap());
        let m = Matcher::new(&["re:help"]).unwrap();
        assert!(m.matches(r"C:\x\My Helper.exe").unwrap());
        assert!(!m.matches(r"C:\help\other.exe").unwrap());
    }

    #[test]
    fn regex_entries_support_lookaround() {
        let m = Matcher::new(&[r"re:^(?!chrome).*\.exe$"]).unwrap();
        assert!(m.matches(r"C:\x\firefox.exe").unwrap());
        assert!(!m.matches(r"C:\x\chrome.exe").unwrap());
    }

    #[test]
    fn a_regex_that_fails_at_run_time_is_an_error() {
        // The pattern needs more backtracking than the default limit allows.
        let m = Matcher::new(&[r"re:^(a+)+\1b"]).unwrap();
        assert!(m.matches(&format!(r"C:\x\{}c", "a".repeat(40))).is_err());
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
                .unwrap()
        );
    }

    #[test]
    fn empty_entries_are_ignored() {
        assert!(Matcher::new(&["", "  "]).unwrap().is_empty());
    }
}
