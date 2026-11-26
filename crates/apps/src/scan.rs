use std::path::{Path, PathBuf};

use heronite_data::apps::{AppCandidate, AppOrigin};

use crate::blacklist::Blacklist;
use crate::InstalledProgram;

#[cfg(windows)]
pub mod windows;

const MAX_DEPTH: usize = 8;

/// Load the app blacklist from the data directory.
pub fn load_blacklist() -> Option<Blacklist> {
    let data_dir = std::env::var("HERONITE_DATA_DIR").ok().map(PathBuf::from).or_else(|| {
        dirs::data_dir().map(|dir| dir.join("Heronite"))
    });
    data_dir.and_then(|dir| {
        let path = dir.join("app-blacklist.json");
        Blacklist::load(&path).ok()
    })
}

pub fn start_menu(roots: &[PathBuf]) -> Vec<AppCandidate> {
    let mut candidates = Vec::new();
    for root in roots {
        visit(root, 0, &mut candidates);
    }
    candidates
}

fn visit(dir: &Path, depth: usize, candidates: &mut Vec<AppCandidate>) {
    if depth > MAX_DEPTH {
        return;
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) => {
            log::debug!("Apps scanner: cannot read {}: {error}", dir.display());
            return;
        }
    };
    let mut paths: Vec<PathBuf> = entries.filter_map(Result::ok).map(|entry| entry.path()).collect();
    paths.sort();
    for path in paths {
        let Ok(metadata) = std::fs::symlink_metadata(&path) else { continue };
        if metadata.file_type().is_symlink() {
            continue;
        }
        if metadata.is_dir() {
            visit(&path, depth + 1, candidates);
            continue;
        }
        match path.extension().and_then(|s| s.to_str()).map(str::to_ascii_lowercase).as_deref() {
            Some("url") => {
                if let Some(candidate) = shortcut::url(&path) {
                    candidates.push(candidate);
                }
            }
            #[cfg(windows)]
            Some("lnk") => {
                if let Some(candidate) = shortcut::link(&path) {
                    candidates.push(candidate);
                }
            }
            _ => {}
        }
    }
}

pub fn attach_uninstall_metadata(candidates: &mut [AppCandidate], entries: &[InstalledProgram]) {
    for candidate in candidates {
        let Some(exe) = candidate.exe.as_deref() else { continue };
        let exe_key = heronite_data::apps::canonical_path(exe);
        let matched = entries
            .iter()
            .find(|entry| {
                entry
                    .display_icon
                    .as_deref()
                    .is_some_and(|icon| icon_path(icon).is_some_and(|path| heronite_data::apps::canonical_path(&path) == exe_key))
            })
            .or_else(|| {
                entries.iter().find(|entry| {
                    entry.install_location.as_deref().is_some_and(|location| {
                        let location = heronite_data::apps::canonical_path(location);
                        heronite_data::apps::game_specific_directory(&location) && exe_key.starts_with(&(location + "\\"))
                    })
                })
            })
            .or_else(|| {
                let name = heronite_data::fuzzy::fold(candidate.name.trim());
                entries.iter().find(|entry| !name.is_empty() && heronite_data::fuzzy::fold(entry.display_name.trim()) == name)
            });
        let Some(entry) = matched else { continue };
        candidate.publisher = candidate.publisher.take().or_else(|| entry.publisher.clone());
        candidate.install_dir = candidate.install_dir.take().or_else(|| entry.install_location.clone());
        if let Some(icon) = entry.display_icon.as_deref().and_then(icon_path) {
            candidate.icon = candidate.icon.take().or(Some(icon));
        }
        candidate.origins.insert(AppOrigin::InstalledProgram);
    }
}

fn icon_path(display_icon: &str) -> Option<String> {
    let trimmed = display_icon.trim();
    let (path, suffix) = if let Some(rest) = trimmed.strip_prefix('"') {
        let end = rest.find('"')?;
        (rest[..end].to_string(), &rest[end + 1..])
    } else {
        match trimmed.rsplit_once(',') {
            Some((path, index)) if index.trim().parse::<i32>().is_ok() => (path.trim().into(), ""),
            _ => (trimmed.into(), ""),
        }
    };
    let _ = suffix;
    (!path.trim().is_empty()).then_some(path)
}

mod shortcut {
    use std::path::Path;

    use heronite_data::apps::{store_url_refs, AppCandidate, AppLaunch, AppOrigin};

    pub(super) fn url(path: &Path) -> Option<AppCandidate> {
        let contents = std::fs::read_to_string(path).ok()?;
        let mut in_section = false;
        let mut url = None;
        for line in contents.lines().map(str::trim) {
            if line.starts_with('[') && line.ends_with(']') {
                in_section = line[1..line.len() - 1].eq_ignore_ascii_case("InternetShortcut");
            } else if in_section {
                if let Some((key, value)) = line.split_once('=') {
                    if key.trim().eq_ignore_ascii_case("URL") {
                        url = Some(value.trim().to_string());
                        break;
                    }
                }
            }
        }
        from_url(path, url?)
    }

    fn from_url(path: &Path, url: String) -> Option<AppCandidate> {
        let lower = url.to_ascii_lowercase();
        if lower.starts_with("http://") || lower.starts_with("https://") {
            return None;
        }
        let (is_store, refs) = store_url_refs(&url);
        if !is_store {
            return None;
        }
        let name = path.file_stem()?.to_string_lossy().trim().to_string();
        if junk_name(&name) {
            return None;
        }
        let mut candidate = AppCandidate::new(name, AppLaunch { target: url, ..Default::default() }, AppOrigin::StartMenu);
        candidate.store_refs = refs;
        candidate.store_scheme_url = true;
        Some(candidate)
    }

    fn junk_name(name: &str) -> bool {
        let name = name.trim().to_ascii_lowercase();
        name.starts_with("uninstall") || name.starts_with("unins")
    }

    #[cfg(windows)]
    pub(super) fn link(path: &Path) -> Option<AppCandidate> {
        let link = match heronite_platform::shortcut::read_resolved(path) {
            Ok(link) => link,
            Err(error) => {
                log::debug!("Apps scanner: cannot resolve {}: {error}", path.display());
                return None;
            }
        };
        let target = link.target.to_string_lossy().trim().to_string();
        if target.is_empty() || link.target.is_dir() || junk_target(&link.target) {
            return None;
        }
        let name = path.file_stem()?.to_string_lossy().trim().to_string();
        let args = link.arguments.trim_start().to_ascii_lowercase();
        if junk_name(&name) || args.starts_with("http://") || args.starts_with("https://") {
            return None;
        }
        let exe = link.target.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("exe")).then(|| target.clone());
        let mut candidate = AppCandidate::new(
            name,
            AppLaunch {
                target: target.clone(),
                shortcut: Some(path.to_string_lossy().into_owned()),
                arguments: link.arguments,
                working_dir: link.working_dir.map(|dir| dir.to_string_lossy().into_owned()),
                ..Default::default()
            },
            AppOrigin::StartMenu,
        );
        candidate.exe = exe;
        candidate.install_dir = link.target.parent().map(|dir| dir.to_string_lossy().into_owned());
        candidate.icon = link.icon.as_ref().map(|(icon, _)| icon.to_string_lossy().into_owned());
        candidate.icon_index = link.icon.map(|(_, index)| index);
        Some(candidate)
    }

    #[cfg(windows)]
    fn junk_target(path: &Path) -> bool {
        let extension = path.extension().and_then(|e| e.to_str()).unwrap_or_default().to_ascii_lowercase();
        if matches!(extension.as_str(), "chm" | "txt" | "pdf" | "html" | "htm") {
            return true;
        }
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or_default().to_ascii_lowercase();
        stem.starts_with("unins") || stem.starts_with("uninstall")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_files_are_ini_parsed_and_web_links_are_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let store = dir.path().join("Game.url");
        std::fs::write(&store, "[InternetShortcut]\nURL=steam://rungameid/570\n").unwrap();
        let web = dir.path().join("Web.url");
        std::fs::write(&web, "[InternetShortcut]\nURL=https://example.com\n").unwrap();
        let malformed = dir.path().join("Bad.url");
        std::fs::write(&malformed, "[Other]\nURL=steam://rungameid/570\n").unwrap();
        let scanned = start_menu(&[dir.path().to_path_buf()]);
        assert_eq!(scanned.len(), 1);
        assert!(scanned[0].store_scheme_url);
        assert_eq!(scanned[0].store_refs[0].id, "570");
    }

    #[test]
    fn icon_path_strips_registry_index_and_preserves_quoted_paths() {
        assert_eq!(icon_path(r#""C:\Apps\My app.exe",0"#).as_deref(), Some(r"C:\Apps\My app.exe"));
        assert_eq!(icon_path(r"C:\Apps\app.exe,4").as_deref(), Some(r"C:\Apps\app.exe"));
    }
}
