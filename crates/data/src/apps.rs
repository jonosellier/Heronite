//! Apps imported from this PC, and the game references used to keep them out
//! of the games Collection.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::json_store;
use crate::models::Extra;
use crate::paths::DataPaths;

pub const SOURCE: &str = "apps";
pub const LAUNCH_KIND: &str = "app";
pub const NAME: &str = "Apps";
pub const CANDIDATE_KEY: &str = "heroniteAppCandidate";

#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoreRef {
    pub store: String,
    pub id: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppLaunch {
    /// Executable path or URL for shortcut launches.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub target: String,
    /// The shortcut to pass to Explorer, when found in the Start Menu.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shortcut: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub arguments: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_dir: Option<String>,
    /// `<package family>!<application id>` for packaged applications.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aumid: Option<String>,
}

impl AppLaunch {
    pub fn canonical_key(&self) -> String {
        if let Some(aumid) = self.aumid.as_deref().filter(|s| !s.trim().is_empty()) {
            return format!("aumid:{}|{}", aumid.to_lowercase(), self.arguments.trim());
        }
        format!("{}|{}", canonical_path(&self.target), self.arguments.trim())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AppOrigin {
    StartMenu,
    InstalledProgram,
    Packaged,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppCandidate {
    /// Stable hash of the canonical launch key.
    #[serde(default)]
    pub id: String,
    pub name: String,
    pub launch: AppLaunch,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exe: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub install_dir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publisher: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_index: Option<i32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub store_refs: Vec<StoreRef>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub origins: BTreeSet<AppOrigin>,
    /// True when a `.url` target uses a known game-store protocol.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub store_scheme_url: bool,
}

impl AppCandidate {
    pub fn refresh_id(&mut self) {
        self.id = stable_id(&self.launch.canonical_key());
    }

    pub fn new(name: impl Into<String>, launch: AppLaunch, origin: AppOrigin) -> Self {
        let mut candidate = Self { name: name.into(), launch, origins: [origin].into_iter().collect(), ..Default::default() };
        candidate.refresh_id();
        candidate
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GameRef {
    pub store: String,
    pub title: String,
    pub install_dir: Option<String>,
    pub exe: Option<String>,
    pub store_ids: BTreeSet<String>,
    pub aumid: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GameMatch {
    StoreRef { store: String, id: String },
    StoreScheme,
    Aumid,
    Executable,
    InstallDirectory,
    Title,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Classification {
    App,
    Game(GameMatch),
}

/// Parse known game-store URL schemes. A recognized scheme is still marked
/// as a game reference when its id cannot be extracted.
pub fn store_url_refs(url: &str) -> (bool, Vec<StoreRef>) {
    let Some((scheme, rest)) = url.trim().split_once("://") else {
        return (false, Vec::new());
    };
    let store = match scheme.to_ascii_lowercase().as_str() {
        "steam" => "steam",
        "com.epicgames.launcher" => "epic",
        "goggalaxy" => "gog",
        "uplay" => "ubisoft",
        "battlenet" => "battlenet",
        _ => return (false, Vec::new()),
    };
    let path = rest.split(['?', '#']).next().unwrap_or_default().trim_matches('/');
    let id = match store {
        "steam" => path.strip_prefix("rungameid/").filter(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())).map(str::to_string),
        "epic" => path.strip_prefix("apps/").and_then(first_segment),
        "gog" => query_value(rest, "id").or_else(|| last_segment(path)),
        "ubisoft" => path.strip_prefix("launch/").and_then(first_segment),
        "battlenet" => first_segment(path),
        _ => None,
    };
    let refs = id.into_iter().map(|id| StoreRef { store: store.into(), id }).collect();
    (true, refs)
}

fn first_segment(s: &str) -> Option<String> {
    s.split('/').next().filter(|s| !s.is_empty()).map(str::to_string)
}

fn last_segment(s: &str) -> Option<String> {
    s.split('/').filter(|s| !s.is_empty()).next_back().map(str::to_string)
}

fn query_value(url: &str, key: &str) -> Option<String> {
    url.split_once('?')?
        .1
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .find(|(k, _)| k.eq_ignore_ascii_case(key))
        .map(|(_, v)| v.to_string())
        .filter(|v| !v.is_empty())
}

pub fn classify(
    candidate: &AppCandidate,
    games: &[GameRef],
    force_includes: &BTreeSet<String>,
    show_title_matches: &BTreeSet<String>,
) -> Classification {
    if force_includes.contains(&candidate.id) {
        return Classification::App;
    }
    for candidate_ref in &candidate.store_refs {
        if games.iter().any(|game| {
            normalized_store(&game.store) == normalized_store(&candidate_ref.store)
                && game.store_ids.iter().any(|id| id.eq_ignore_ascii_case(&candidate_ref.id))
        }) {
            return Classification::Game(GameMatch::StoreRef { store: candidate_ref.store.clone(), id: candidate_ref.id.clone() });
        }
    }
    if candidate.store_scheme_url || !candidate.store_refs.is_empty() {
        return Classification::Game(GameMatch::StoreScheme);
    }
    for game in games {
        if let (Some(aumid), Some(game_aumid)) = (candidate.launch.aumid.as_deref(), game.aumid.as_deref()) {
            if aumid.eq_ignore_ascii_case(game_aumid) {
                return Classification::Game(GameMatch::Aumid);
            }
        }
    }
    for game in games {
        if let (Some(exe), Some(game_exe)) = (candidate.exe.as_deref(), game.exe.as_deref()) {
            if canonical_path(exe) == canonical_path(game_exe) {
                return Classification::Game(GameMatch::Executable);
            }
        }
    }
    for game in games {
        if let (Some(exe), Some(dir)) = (candidate.exe.as_deref(), game.install_dir.as_deref()) {
            let exe = canonical_path(exe);
            let dir = canonical_path(dir);
            if game_specific_directory(&dir) && exe.starts_with(&(dir + "\\")) {
                return Classification::Game(GameMatch::InstallDirectory);
            }
        }
    }
    for game in games {
        if crate::fuzzy::fold(candidate.name.trim()) == crate::fuzzy::fold(game.title.trim())
            && !candidate.name.trim().is_empty()
            && !game.title.trim().is_empty()
        {
            return if show_title_matches.contains(&candidate.id) { Classification::App } else { Classification::Game(GameMatch::Title) };
        }
    }
    Classification::App
}

fn normalized_store(store: &str) -> String {
    match store.to_ascii_lowercase().as_str() {
        "goggalaxy" => "gog".into(),
        "uplay" => "ubisoft".into(),
        "com.epicgames.launcher" => "epic".into(),
        "battle.net" => "battlenet".into(),
        store => store.into(),
    }
}

pub fn game_specific_directory(dir: &str) -> bool {
    let parts: Vec<&str> = dir.split('\\').filter(|part| !part.is_empty()).collect();
    let Some(last) = parts.last() else { return false };
    if parts.len() < 3 {
        return false;
    }
    !matches!(
        *last,
        "program files"
            | "program files (x86)"
            | "games"
            | "windows"
            | "programdata"
            | "users"
            | "common"
            | "steamapps"
            | "windowsapps"
            | "apps"
    )
}

pub fn canonical_path(path: &str) -> String {
    let expanded = expand_environment(path).replace('/', "\\");
    let without_verbatim = expanded.strip_prefix(r"\\?\").unwrap_or(&expanded);
    let lower = without_verbatim.to_lowercase();
    let unc = lower.starts_with(r"\\");
    let drive = lower.get(1..2) == Some(":");
    let prefix_len = if unc {
        2
    } else if drive {
        2
    } else {
        0
    };
    let prefix = &lower[..prefix_len];
    let mut parts: Vec<&str> = Vec::new();
    for part in lower[prefix_len..].split('\\').filter(|part| !part.is_empty() && *part != ".") {
        if part == ".." {
            if !parts.is_empty() && parts.last() != Some(&"..") {
                parts.pop();
            } else if !drive && !unc {
                parts.push(part);
            }
        } else {
            parts.push(part);
        }
    }
    let joined = parts.join("\\");
    if prefix.is_empty() {
        joined
    } else if drive {
        let value = format!("{prefix}\\{joined}");
        if joined.is_empty() {
            value
        } else {
            value.trim_end_matches('\\').to_string()
        }
    } else if joined.is_empty() {
        prefix.to_string()
    } else {
        format!("{prefix}{joined}")
    }
}

fn expand_environment(path: &str) -> String {
    if !path.contains('%') {
        return path.to_string();
    }
    let mut result = String::with_capacity(path.len());
    let mut rest = path;
    while let Some(start) = rest.find('%') {
        result.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let Some(end) = after.find('%') else {
            result.push_str(&rest[start..]);
            return result;
        };
        let name = &after[..end];
        let value = std::env::var(name)
            .ok()
            .or_else(|| std::env::vars().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v));
        if let Some(value) = value {
            result.push_str(&value);
        } else {
            result.push('%');
            result.push_str(name);
            result.push('%');
        }
        rest = &after[end + 1..];
    }
    result.push_str(rest);
    result
}

fn stable_id(key: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(key.as_bytes());
    let mut id = String::with_capacity(2 + 24);
    id.push_str("app_");
    for byte in &digest[..12] {
        use std::fmt::Write as _;
        let _ = write!(id, "{byte:02x}");
    }
    id
}

pub fn dedupe(candidates: impl IntoIterator<Item = AppCandidate>) -> Vec<AppCandidate> {
    let mut by_key: BTreeMap<String, AppCandidate> = BTreeMap::new();
    for mut candidate in candidates {
        let key = candidate.launch.canonical_key();
        candidate.refresh_id();
        match by_key.get_mut(&key) {
            Some(existing) => {
                existing.origins.append(&mut candidate.origins);
                for store_ref in candidate.store_refs {
                    if !existing.store_refs.contains(&store_ref) {
                        existing.store_refs.push(store_ref);
                    }
                }
                existing.store_scheme_url |= candidate.store_scheme_url;
                existing.exe = existing.exe.take().or(candidate.exe);
                existing.install_dir = existing.install_dir.take().or(candidate.install_dir);
                existing.publisher = existing.publisher.take().or(candidate.publisher);
                existing.icon = existing.icon.take().or(candidate.icon);
                existing.icon_index = existing.icon_index.or(candidate.icon_index);
            }
            None => {
                by_key.insert(key, candidate);
            }
        }
    }
    by_key.into_values().collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RuleAction {
    Hide,
    Allow,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RuleField {
    #[default]
    Any,
    Name,
    Publisher,
    Executable,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppRule {
    pub pattern: String,
    #[serde(default)]
    pub field: RuleField,
    pub action: RuleAction,
}

const DEFAULT_HIDE_NAME_PATTERNS: &[&str] = &[
    "steam",
    "epic games launcher",
    "gog galaxy",
    "ubisoft connect",
    "battle.net",
    "ea app",
    "rockstar games launcher",
    "amazon games",
    "windows terminal",
    "command prompt",
    "cmd.exe",
    "powershell",
    "powershell ise",
    "windows script host",
    "windows shell",
    "bash",
    "zsh",
    "wsl",
    "registry editor",
    "task manager",
    "control panel",
    "services",
    "device manager",
    "event viewer",
    "resource monitor",
    "windows tools",
    "system tools",
    "system settings",
    "windows security",
    "windows defender",
    "windows update",
    "device setup",
    "driver",
    "sdk",
    "visual c++",
    "redistributable",
    ".net runtime",
    ".net desktop runtime",
    "asp.net core runtime",
    "microsoft edge webview",
    "webview2 runtime",
    "directx runtime",
    "vulkan runtime",
    "opengl",
    "openal",
    "physx",
    "runtime",
    "uninstall",
    "setup",
    "maintenance",
    "update service",
];

fn folded_default_rules() -> &'static [(String, RuleField)] {
    static RULES: std::sync::OnceLock<Vec<(String, RuleField)>> = std::sync::OnceLock::new();
    RULES.get_or_init(|| {
        default_rules()
            .into_iter()
            .filter_map(|rule| {
                let needle = crate::fuzzy::fold(rule.pattern.trim());
                (!needle.is_empty()).then_some((needle, rule.field))
            })
            .collect()
    })
}

pub fn default_rules() -> Vec<AppRule> {
    DEFAULT_HIDE_NAME_PATTERNS
        .iter()
        .map(|name| {
            let field = if *name == "cmd.exe" { RuleField::Any } else { RuleField::Name };
            AppRule { pattern: (*name).into(), field, action: RuleAction::Hide }
        })
        .collect()
}

pub fn is_hidden(candidate: &AppCandidate, prefs: &AppPrefs) -> bool {
    if prefs.allow_overrides.contains(&candidate.id) {
        return false;
    }
    for rule in prefs.rules.iter().filter(|rule| rule.action == RuleAction::Allow) {
        if rule_matches(rule, candidate) {
            return false;
        }
    }
    if prefs.rules.iter().filter(|rule| rule.action == RuleAction::Hide).any(|rule| rule_matches(rule, candidate)) {
        return true;
    }
    let name_folded = crate::fuzzy::fold(&candidate.name);
    let mut any_folded: Option<[String; 2]> = None;
    folded_default_rules().iter().any(|(needle, field)| match field {
        RuleField::Name => name_folded.contains(needle),
        _ => {
            let [publisher, exe] = any_folded.get_or_insert_with(|| {
                [
                    crate::fuzzy::fold(candidate.publisher.as_deref().unwrap_or_default()),
                    crate::fuzzy::fold(candidate.exe.as_deref().unwrap_or_default()),
                ]
            });
            name_folded.contains(needle) || publisher.contains(needle) || exe.contains(needle)
        }
    })
}

/// Whether an imported app belongs in a main Collection view. The Apps
/// store filter can reveal apps when they are not shown alongside games.
pub fn visible_in_main_library(game: &crate::library::CollectionGame, prefs: &AppPrefs, apps_filter: bool) -> bool {
    if game.launch_kind.as_deref() != Some(LAUNCH_KIND) {
        return true;
    }
    if !prefs.enabled || prefs.hidden_ids.contains(&game.app_name) {
        return false;
    }
    if !prefs.show_in_main_library && !apps_filter {
        return false;
    }
    if let Some(candidate) = game
        .meta
        .as_deref()
        .and_then(|meta| meta.extra.get(CANDIDATE_KEY))
        .and_then(|value| serde_json::from_value::<AppCandidate>(value.clone()).ok())
    {
        if is_hidden(&candidate, prefs) {
            return false;
        }
    }
    true
}

fn rule_matches(rule: &AppRule, candidate: &AppCandidate) -> bool {
    let needle = crate::fuzzy::fold(rule.pattern.trim());
    if needle.is_empty() {
        return false;
    }
    let fields = match rule.field {
        RuleField::Name => vec![candidate.name.as_str()],
        RuleField::Publisher => candidate.publisher.as_deref().into_iter().collect(),
        RuleField::Executable => candidate.exe.as_deref().into_iter().collect(),
        RuleField::Any => {
            vec![candidate.name.as_str(), candidate.publisher.as_deref().unwrap_or_default(), candidate.exe.as_deref().unwrap_or_default()]
        }
    };
    fields.into_iter().any(|field| crate::fuzzy::fold(field).contains(&needle))
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LastSync {
    pub at: String,
    pub apps: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppPrefs {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub show_in_main_library: bool,
    #[serde(default)]
    pub show_badges: bool,
    #[serde(default = "yes")]
    pub import_start_menu: bool,
    #[serde(default = "yes")]
    pub import_installed_programs: bool,
    #[serde(default = "yes")]
    pub import_packaged: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last: Option<LastSync>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub hidden_ids: BTreeSet<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rules: Vec<AppRule>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub allow_overrides: BTreeSet<String>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub force_includes: BTreeSet<String>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub show_title_matches: BTreeSet<String>,
    #[serde(flatten)]
    pub extra: Extra,
}

fn yes() -> bool {
    true
}

impl Default for AppPrefs {
    fn default() -> Self {
        Self {
            enabled: false,
            show_in_main_library: false,
            show_badges: false,
            import_start_menu: true,
            import_installed_programs: true,
            import_packaged: true,
            last: None,
            hidden_ids: BTreeSet::new(),
            rules: Vec::new(),
            allow_overrides: BTreeSet::new(),
            force_includes: BTreeSet::new(),
            show_title_matches: BTreeSet::new(),
            extra: Extra::new(),
        }
    }
}

impl AppPrefs {
    pub fn load(paths: &DataPaths) -> Self {
        json_store::read(&paths.apps()).unwrap_or_default()
    }

    pub fn save(&self, paths: &DataPaths) -> Result<()> {
        json_store::write(&paths.apps(), self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(name: &str, exe: Option<&str>) -> AppCandidate {
        let mut candidate =
            AppCandidate::new(name, AppLaunch { target: exe.unwrap_or_default().into(), ..Default::default() }, AppOrigin::StartMenu);
        candidate.exe = exe.map(str::to_string);
        candidate
    }

    fn game(title: &str) -> GameRef {
        GameRef { title: title.into(), ..Default::default() }
    }

    #[test]
    fn store_urls_are_references_and_unmatched_store_urls_are_not_apps() {
        let (is_store, refs) = store_url_refs("steam://rungameid/570");
        assert!(is_store);
        assert_eq!(refs, [StoreRef { store: "steam".into(), id: "570".into() }]);
        let mut app = candidate("Dota 2", None);
        app.store_refs = refs;
        let games = [GameRef { store: "steam".into(), store_ids: ["570".into()].into(), ..game("Dota 2") }];
        assert!(matches!(classify(&app, &games, &BTreeSet::new(), &BTreeSet::new()), Classification::Game(GameMatch::StoreRef { .. })));

        let (is_store, refs) = store_url_refs("steam://rungameid/99999");
        let mut unmatched = candidate("Mystery", None);
        unmatched.store_refs = refs;
        unmatched.store_scheme_url = is_store;
        assert_eq!(classify(&unmatched, &[], &BTreeSet::new(), &BTreeSet::new()), Classification::Game(GameMatch::StoreScheme));
    }

    #[test]
    fn store_schemes_extract_supported_ids() {
        for (url, store, id) in [
            ("com.epicgames.launcher://apps/Fortnite?action=launch", "epic", "Fortnite"),
            ("goggalaxy://openGameView?id=12345", "gog", "12345"),
            ("uplay://launch/123/0", "ubisoft", "123"),
            ("battlenet://StarCraft", "battlenet", "StarCraft"),
        ] {
            let (recognized, refs) = store_url_refs(url);
            assert!(recognized, "{url}");
            assert_eq!(refs, [StoreRef { store: store.into(), id: id.into() }], "{url}");
        }
    }

    #[test]
    fn matching_guards_generic_install_directories_and_supports_game_specific_ones() {
        let app = candidate("Utility", Some(r"C:\Games\Different Game\utility.exe"));
        let shared = [GameRef { install_dir: Some(r"C:\Games".into()), ..game("Different") }];
        assert_eq!(classify(&app, &shared, &BTreeSet::new(), &BTreeSet::new()), Classification::App);

        let specific = [GameRef { install_dir: Some(r"C:\Games\Different Game".into()), ..game("Different") }];
        assert_eq!(classify(&app, &specific, &BTreeSet::new(), &BTreeSet::new()), Classification::Game(GameMatch::InstallDirectory));
    }

    #[test]
    fn exact_executable_and_aumid_matches_are_stronger_than_title() {
        let mut app = candidate("Game", Some(r"C:\Apps\game.exe"));
        let games = [GameRef { title: "Not the same title".into(), exe: Some(r"c:/apps/./GAME.exe".into()), ..Default::default() }];
        assert_eq!(classify(&app, &games, &BTreeSet::new(), &BTreeSet::new()), Classification::Game(GameMatch::Executable));

        app.exe = None;
        app.launch.aumid = Some("Package_Family!App".into());
        let games = [GameRef { aumid: Some("package_family!app".into()), ..game("Other") }];
        assert_eq!(classify(&app, &games, &BTreeSet::new(), &BTreeSet::new()), Classification::Game(GameMatch::Aumid));
    }

    #[test]
    fn title_only_matches_can_be_shown_anyway() {
        let app = candidate("Chrome", None);
        let games = [game("CHROME")];
        assert_eq!(classify(&app, &games, &BTreeSet::new(), &BTreeSet::new()), Classification::Game(GameMatch::Title));
        assert_eq!(classify(&app, &games, &BTreeSet::new(), &[app.id.clone()].into()), Classification::App);
    }

    #[test]
    fn dedupe_merges_origins_and_registry_metadata_for_same_launch() {
        let mut menu = candidate("Example", Some(r"C:\Program Files\Example\app.exe"));
        let mut registry = candidate("Example", Some(r"c:/program files/example/./APP.EXE"));
        menu.launch.arguments = "--open".into();
        registry.launch.arguments = "--open".into();
        registry.origins.insert(AppOrigin::InstalledProgram);
        registry.publisher = Some("Publisher".into());
        let entries = dedupe([menu, registry]);
        assert_eq!(entries.len(), 1);
        assert!(entries[0].origins.contains(&AppOrigin::InstalledProgram));
        assert_eq!(entries[0].publisher.as_deref(), Some("Publisher"));
    }

    #[test]
    fn prefs_default_sources_on_but_import_and_visibility_off_and_unknown_fields_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        let default = AppPrefs::load(&paths);
        assert!(!default.enabled);
        assert!(!default.show_in_main_library);
        assert!(!default.show_badges);
        assert!(default.import_start_menu && default.import_installed_programs && default.import_packaged);
        std::fs::create_dir_all(paths.apps().parent().unwrap()).unwrap();
        std::fs::write(paths.apps(), r#"{"enabled":true,"futureKey":7}"#).unwrap();
        let prefs = AppPrefs::load(&paths);
        assert!(prefs.enabled);
        prefs.save(&paths).unwrap();
        assert!(std::fs::read_to_string(paths.apps()).unwrap().contains("\"futureKey\": 7"));
    }

    #[test]
    fn default_blacklist_hides_clients_and_user_allow_rules_override_it() {
        let app = candidate("Steam", Some(r"C:\Program Files (x86)\Steam\steam.exe"));
        let mut prefs = AppPrefs::default();
        assert!(is_hidden(&app, &prefs));
        prefs.allow_overrides.insert(app.id.clone());
        assert!(!is_hidden(&app, &prefs));
    }

    #[test]
    fn default_blacklist_hides_runtimes_shells_and_system_tools() {
        let prefs = AppPrefs::default();
        for name in ["Microsoft Visual C++ 2015 Redistributable (x64)", "Windows PowerShell", "Command Prompt", "Device Manager"] {
            assert!(is_hidden(&candidate(name, None), &prefs), "{name}");
        }
        assert!(!is_hidden(&candidate("My Audio Tool", Some(r"C:\Tools\Runtime\Driver\Setup\audio.exe")), &prefs));
    }

    #[test]
    fn canonical_paths_remove_verbatim_prefix_and_normalize_separators() {
        assert_eq!(canonical_path(r"\\?\C:/Games/../Apps/Game.exe"), r"c:\apps\game.exe");
        assert_eq!(canonical_path(r"C:\Games"), r"c:\games");
    }

    #[test]
    fn visible_in_main_library_respects_enabled_hidden_and_store_filter() {
        let app = crate::library::tests_support::game_with("app_1", "sideload", |_, _, _, meta, _| {
            meta.source = SOURCE.into();
            meta.launch_kind = LAUNCH_KIND.into();
        });
        assert!(!visible_in_main_library(&app, &AppPrefs::default(), false));
        let enabled = AppPrefs { enabled: true, ..Default::default() };
        assert!(!visible_in_main_library(&app, &enabled, false));
        assert!(visible_in_main_library(&app, &enabled, true));
        let in_main = AppPrefs { enabled: true, show_in_main_library: true, ..Default::default() };
        assert!(visible_in_main_library(&app, &in_main, false));
        let hidden = AppPrefs { hidden_ids: ["app_1".into()].into(), ..in_main };
        assert!(!visible_in_main_library(&app, &hidden, true));
    }
}
