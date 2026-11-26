//! App blacklist filtering based on the JSON configuration.

use std::collections::BTreeMap;
use std::path::{ Path, PathBuf };

use anyhow::Result;
use heronite_data::apps::AppCandidate;
use serde::{ Deserialize, Serialize };

/// The blacklist configuration loaded from JSON.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BlacklistConfig {
    #[serde(default)]
    pub matching: MatchingConfig,
    #[serde(default)]
    pub groups: BTreeMap<String, GroupConfig>,
    #[serde(default)]
    pub allow: Vec<Rule>,
    #[serde(default)]
    pub rules: Vec<Rule>,
}

/// Matching configuration options.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MatchingConfig {
    #[serde(default)]
    pub case_sensitive: bool,
}

/// Group configuration.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub description: String,
}

fn default_true() -> bool {
    true
}

/// A single blacklist/allow rule.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Rule {
    pub id: String,
    #[serde(default)]
    pub group: String,
    pub field: String,
    #[serde(default = "default_match_type")]
    pub match_type: MatchType,
    pub patterns: Vec<String>,
    #[serde(default)]
    pub reason: String,
}

fn default_match_type() -> MatchType {
    MatchType::Fuzzy
}

/// Match type for patterns.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MatchType {
    #[default]
    Fuzzy,
    Exact,
}

/// Fields that can be matched against.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum CandidateField {
    Title,
    Aumid,
    Target,
    Arguments,
    Shortcut,
    InstallDir,
    Publisher,
    Exe,
}

impl CandidateField {
    /// Get the field value from a candidate.
    pub fn get_value(&self, candidate: &AppCandidate) -> Option<String> {
        match self {
            CandidateField::Title => Some(candidate.name.clone()),
            CandidateField::Aumid => candidate.launch.aumid.clone(),
            CandidateField::Target => Some(candidate.launch.target.clone()),
            CandidateField::Arguments => Some(candidate.launch.arguments.clone()),
            CandidateField::Shortcut => candidate.launch.shortcut.clone(),
            CandidateField::InstallDir => candidate.install_dir.clone(),
            CandidateField::Publisher => candidate.publisher.clone(),
            CandidateField::Exe => candidate.exe.clone(),
        }
    }

    /// Parse a field name from the JSON config.
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "title" => Some(CandidateField::Title),
            "aumid" => Some(CandidateField::Aumid),
            "target" => Some(CandidateField::Target),
            "arguments" => Some(CandidateField::Arguments),
            "shortcut" => Some(CandidateField::Shortcut),
            "installdir" => Some(CandidateField::InstallDir),
            "publisher" => Some(CandidateField::Publisher),
            "exe" => Some(CandidateField::Exe),
            _ => None,
        }
    }
}

/// Normalize a path for matching (convert backslashes to forward slashes).
fn normalize_path(path: &str) -> String {
    path.replace('\\', "/")
}

/// Check if a value matches a pattern according to the match type.
fn matches_pattern(
    value: &str,
    pattern: &str,
    match_type: MatchType,
    case_sensitive: bool
) -> bool {
    let (value, pattern) = if case_sensitive {
        (value.to_string(), pattern.to_string())
    } else {
        (value.to_lowercase(), pattern.to_lowercase())
    };

    match match_type {
        MatchType::Exact => value == pattern,
        MatchType::Fuzzy => {
            if pattern.contains('*') {
                // Wildcard matching: * matches any sequence of characters
                wildcard_match(&value, &pattern)
            } else {
                // Substring matching
                value.contains(&pattern)
            }
        }
    }
}

/// Simple wildcard matching where * matches any sequence of characters.
fn wildcard_match(text: &str, pattern: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.is_empty() {
        return true;
    }

    let mut text_idx = 0;

    // Check prefix (before first *)
    if !pattern.starts_with('*') {
        let prefix = parts[0];
        if !text[text_idx..].starts_with(prefix) {
            return false;
        }
        text_idx += prefix.len();
    }

    // Check middle parts
    for part in &parts[1..parts.len() - 1] {
        if part.is_empty() {
            continue;
        }
        if let Some(idx) = text[text_idx..].find(part) {
            text_idx += idx + part.len();
        } else {
            return false;
        }
    }

    // Check suffix (after last *)
    if !pattern.ends_with('*') {
        let suffix = parts.last().unwrap();
        if !text.ends_with(suffix) {
            return false;
        }
    }

    true
}

/// Get the default blacklist file path.
pub fn blacklist_path() -> PathBuf {
    // First check the OUT_DIR (for built binaries and tests)
    if let Ok(out_dir) = std::env::var("OUT_DIR") {
        let out_path = PathBuf::from(out_dir).join("app-blacklist.json");
        if out_path.exists() {
            return out_path;
        }
    }

    // First check the project data directory (for development)
    if let Ok(manifest_dir) = std::env::var("CARGO_MANIFEST_DIR") {
        let project_data = PathBuf::from(manifest_dir)
            .join("..")
            .join("..")
            .join("data")
            .join("app_blacklist.json");
        if project_data.exists() {
            return project_data;
        }
    }

    // Also check relative to current directory (for tests)
    if let Ok(cwd) = std::env::current_dir() {
        // Try multiple levels up from target directory
        for i in 0..5 {
            let mut path = cwd.clone();
            for _ in 0..i {
                path = path.join("..");
            }
            path = path.join("data").join("app_blacklist.json");
            if path.exists() {
                return path;
            }
        }
    }

    // Then check the user data directory
    std::env
        ::var("HERONITE_DATA_DIR")
        .ok()
        .map(PathBuf::from)
        .or_else(|| dirs::data_dir().map(|dir| dir.join("Heronite")))
        .map(|dir| dir.join("app-blacklist.json"))
        .unwrap_or_else(|| PathBuf::from("app-blacklist.json"))
}

/// The blacklist engine that applies rules to candidates.
pub struct Blacklist {
    config: BlacklistConfig,
}

impl Blacklist {
    /// Load the blacklist from a JSON file.
    pub fn load(path: &Path) -> Result<Self> {
        log::info!("Loading blacklist from: {}", path.display());
        let content = std::fs::read_to_string(path)?;
        let config: BlacklistConfig = serde_json::from_str(&content)?;
        log::info!(
            "Blacklist loaded: {} groups, {} allow rules, {} block rules",
            config.groups.len(),
            config.allow.len(),
            config.rules.len()
        );
        Ok(Self { config })
    }

    /// Check if a candidate should be filtered out (blacklisted).
    /// Returns true if the candidate should be dropped.
    pub fn is_blacklisted(&self, candidate: &AppCandidate) -> bool {
        // First check allow rules - if any matches, keep the candidate
        for rule in &self.config.allow {
            if self.rule_matches(rule, candidate) {
                log::info!("Candidate '{}' allowed by rule '{}'", candidate.name, rule.id);
                return false;
            }
        }

        // Then check enabled group rules - if any matches, drop the candidate
        for rule in &self.config.rules {
            let group_enabled = self.config.groups
                .get(&rule.group)
                .map(|g| g.enabled)
                .unwrap_or(true);
            if group_enabled && self.rule_matches(rule, candidate) {
                log::info!(
                    "Candidate '{}' blacklisted by rule '{}' (group: {})",
                    candidate.name,
                    rule.id,
                    rule.group
                );
                return true;
            }
        }

        false
    }

    /// Check if a single rule matches a candidate.
    fn rule_matches(&self, rule: &Rule, candidate: &AppCandidate) -> bool {
        let Some(field) = CandidateField::from_str(&rule.field) else {
            log::info!("Rule '{}': unknown field '{}'", rule.id, rule.field);
            return false;
        };

        let Some(value) = field.get_value(candidate) else {
            // Missing field never matches
            log::info!("Rule '{}': candidate missing field '{}'", rule.id, rule.field);
            return false;
        };

        // Normalize paths for path-based fields
        let normalized_value = match field {
            CandidateField::Target | CandidateField::Shortcut | CandidateField::InstallDir => {
                normalize_path(&value)
            }
            _ => value,
        };

        for pattern in &rule.patterns {
            let normalized_pattern = match field {
                CandidateField::Target | CandidateField::Shortcut | CandidateField::InstallDir => {
                    normalize_path(pattern)
                }
                _ => pattern.clone(),
            };

            log::info!(
                "Rule '{}': matching field='{}' value='{}' pattern='{}' type={:?}",
                rule.id,
                rule.field,
                normalized_value,
                normalized_pattern,
                rule.match_type
            );

            if
                matches_pattern(
                    &normalized_value,
                    &normalized_pattern,
                    rule.match_type,
                    self.config.matching.case_sensitive
                )
            {
                log::info!("Rule '{}': MATCHED!", rule.id);
                return true;
            }
        }

        false
    }
}

/// Filter a list of candidates, removing blacklisted ones.
pub fn filter_candidates(
    candidates: Vec<AppCandidate>,
    blacklist: &Blacklist
) -> Vec<AppCandidate> {
    candidates
        .into_iter()
        .filter(|c| !blacklist.is_blacklisted(c))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use heronite_data::apps::{ AppLaunch, AppOrigin };

    fn make_candidate(
        name: &str,
        target: &str,
        aumid: Option<&str>,
        exe: Option<&str>,
        install_dir: Option<&str>,
        publisher: Option<&str>,
        shortcut: Option<&str>
    ) -> AppCandidate {
        let mut candidate = AppCandidate::new(
            name,
            AppLaunch {
                target: target.into(),
                aumid: aumid.map(|s| s.into()),
                shortcut: shortcut.map(|s| s.into()),
                ..Default::default()
            },
            AppOrigin::StartMenu
        );
        candidate.exe = exe.map(|s| s.into());
        candidate.install_dir = install_dir.map(|s| s.into());
        candidate.publisher = publisher.map(|s| s.into());
        candidate
    }

    #[test]
    fn test_wildcard_match() {
        assert!(wildcard_match("hello world", "hello*"));
        assert!(wildcard_match("hello world", "*world"));
        assert!(wildcard_match("hello world", "he*wo*ld"));
        assert!(wildcard_match("hello world", "*"));
        assert!(!wildcard_match("hello world", "goodbye*"));
        assert!(!wildcard_match("hello world", "*goodbye*"));
    }

    #[test]
    fn test_normalize_path() {
        assert_eq!(normalize_path(r"C:\Windows\SystemApps"), "C:/Windows/SystemApps");
        assert_eq!(normalize_path("C:/Windows/SystemApps"), "C:/Windows/SystemApps");
    }

    #[test]
    fn test_field_extraction() {
        let candidate = make_candidate(
            "Test App",
            r"C:\Program Files\Test\app.exe",
            Some("TestPackage!App"),
            Some(r"C:\Program Files\Test\app.exe"),
            Some(r"C:\Program Files\Test"),
            Some("Test Publisher"),
            Some(r"C:\ProgramData\Microsoft\Windows\Start Menu\Programs\Test.lnk")
        );

        assert_eq!(CandidateField::Title.get_value(&candidate), Some("Test App".into()));
        assert_eq!(CandidateField::Aumid.get_value(&candidate), Some("TestPackage!App".into()));
        assert_eq!(
            CandidateField::Target.get_value(&candidate),
            Some(r"C:\Program Files\Test\app.exe".into())
        );
        assert_eq!(
            CandidateField::Exe.get_value(&candidate),
            Some(r"C:\Program Files\Test\app.exe".into())
        );
        assert_eq!(
            CandidateField::InstallDir.get_value(&candidate),
            Some(r"C:\Program Files\Test".into())
        );
        assert_eq!(CandidateField::Publisher.get_value(&candidate), Some("Test Publisher".into()));
        assert_eq!(
            CandidateField::Shortcut.get_value(&candidate),
            Some(r"C:\ProgramData\Microsoft\Windows\Start Menu\Programs\Test.lnk".into())
        );
    }

    #[test]
    fn test_blacklist_unresolved_resource_names() {
        // Test that the blacklist correctly filters out unresolved resource names
        let blacklist = Blacklist::load(&blacklist_path()).expect("Failed to load blacklist");

        // Create a candidate with an unresolved resource name like the one in the issue
        let candidate = make_candidate(
            "@{MicrosoftWindows.Client.AIX_1000.26100.2161.0_x64__cw5n1h2txyewy?ms-resource://MicrosoftWindows.Client.AIX/resources/ProductPkgDisplayName}",
            r"C:\Windows\SystemApps\MicrosoftWindows.Client.AIX_1000.26100.2161.0_x64__cw5n1h2txyewy\App.exe",
            None,
            Some(
                r"C:\Windows\SystemApps\MicrosoftWindows.Client.AIX_1000.26100.2161.0_x64__cw5n1h2txyewy\App.exe"
            ),
            Some(
                r"C:\Windows\SystemApps\MicrosoftWindows.Client.AIX_1000.26100.2161.0_x64__cw5n1h2txyewy"
            ),
            None,
            None
        );

        let is_blacklisted = blacklist.is_blacklisted(&candidate);
        assert!(is_blacklisted, "Candidate with unresolved resource name should be blacklisted");
    }

    #[test]
    fn test_blacklist_system_apps() {
        let blacklist = Blacklist::load(&blacklist_path()).expect("Failed to load blacklist");

        // Create a candidate in SystemApps folder
        let candidate = make_candidate(
            "Some System App",
            r"C:\Windows\SystemApps\Microsoft.Windows.ShellExperienceHost_cw5n1h2txyewy\ShellExperienceHost.exe",
            Some("Microsoft.Windows.ShellExperienceHost_cw5n1h2txyewy!App"),
            Some(
                r"C:\Windows\SystemApps\Microsoft.Windows.ShellExperienceHost_cw5n1h2txyewy\ShellExperienceHost.exe"
            ),
            Some(r"C:\Windows\SystemApps\Microsoft.Windows.ShellExperienceHost_cw5n1h2txyewy"),
            None,
            None
        );

        let is_blacklisted = blacklist.is_blacklisted(&candidate);
        assert!(is_blacklisted, "Candidate in SystemApps should be blacklisted");
    }

    #[test]
    fn test_blacklist_uninstallers() {
        let blacklist = Blacklist::load(&blacklist_path()).expect("Failed to load blacklist");

        // Create a candidate with uninstall in title
        let candidate = make_candidate(
            "Uninstall MyApp",
            r"C:\Program Files\MyApp\uninstall.exe",
            None,
            Some(r"C:\Program Files\MyApp\uninstall.exe"),
            Some(r"C:\Program Files\MyApp"),
            None,
            None
        );

        let is_blacklisted = blacklist.is_blacklisted(&candidate);
        assert!(is_blacklisted, "Uninstaller should be blacklisted");
    }
}
