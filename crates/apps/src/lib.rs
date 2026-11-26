//! Discovering Windows applications from shortcuts and packaged-app records.

pub mod blacklist;
pub mod collection;
pub mod scan;

use std::path::PathBuf;

use heronite_data::apps::{AppCandidate, AppOrigin, AppPrefs};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InstalledProgram {
    pub display_name: String,
    pub publisher: Option<String>,
    pub install_location: Option<String>,
    pub display_icon: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PackagedApplication {
    pub family_name: String,
    pub app_id: String,
    pub name: String,
    pub install_dir: Option<String>,
}

/// All filesystem and registry inputs to a scan. Tests can supply temporary
/// menu folders and an in-memory view of uninstall/package entries.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScanRoots {
    pub start_menu_dirs: Vec<PathBuf>,
    pub installed_programs: Vec<InstalledProgram>,
    pub packaged_apps: Vec<PackagedApplication>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScanOptions {
    pub start_menu: bool,
    /// Registry values enrich shortcuts; they never create app entries.
    pub installed_program_metadata: bool,
    pub packaged: bool,
}

impl From<&AppPrefs> for ScanOptions {
    fn from(prefs: &AppPrefs) -> Self {
        Self {
            start_menu: prefs.import_start_menu,
            installed_program_metadata: prefs.import_installed_programs,
            packaged: prefs.import_packaged,
        }
    }
}

/// Roots for the current Windows user. Other systems have no Windows scanner.
#[cfg(windows)]
pub fn this_pc() -> anyhow::Result<ScanRoots> {
    scan::windows::this_pc()
}

#[cfg(not(windows))]
pub fn this_pc() -> anyhow::Result<ScanRoots> {
    anyhow::bail!("Windows application scanning is available on Windows only")
}

pub fn scan(roots: &ScanRoots, options: ScanOptions) -> Vec<AppCandidate> {
    let mut candidates = Vec::new();
    if options.start_menu {
        candidates.extend(scan::start_menu(&roots.start_menu_dirs));
    }
    if options.packaged {
        candidates.extend(roots.packaged_apps.iter().filter_map(packaged_candidate));
    }
    if options.installed_program_metadata {
        scan::attach_uninstall_metadata(&mut candidates, &roots.installed_programs);
    }
    let candidates = heronite_data::apps::dedupe(candidates);
    // Apply blacklist filtering to all candidates after deduplication
    if let Ok(blacklist) = crate::blacklist::Blacklist::load(&crate::blacklist::blacklist_path()) {
        crate::blacklist::filter_candidates(candidates, &blacklist)
    } else {
        candidates
    }
}

fn packaged_candidate(app: &PackagedApplication) -> Option<AppCandidate> {
    let (family, id) = (app.family_name.trim(), app.app_id.trim());
    if family.is_empty() || id.is_empty() || app.name.trim().is_empty() {
        return None;
    }
    let aumid = format!("{family}!{id}");
    let mut candidate = AppCandidate::new(
        app.name.trim(),
        heronite_data::apps::AppLaunch { aumid: Some(aumid), ..Default::default() },
        AppOrigin::Packaged,
    );
    candidate.install_dir = app.install_dir.clone();
    Some(candidate)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_only_entries_are_not_imported_but_enrich_shortcuts() {
        let dir = tempfile::tempdir().unwrap();
        let shortcut = dir.path().join("Sample.lnk");
        let mut candidate = AppCandidate::new(
            "Sample",
            heronite_data::apps::AppLaunch { target: r"C:\Apps\sample.exe".into(), ..Default::default() },
            AppOrigin::StartMenu,
        );
        candidate.exe = Some(r"C:\Apps\sample.exe".into());
        std::fs::write(&shortcut, "").unwrap();

        let roots = ScanRoots {
            start_menu_dirs: vec![dir.path().to_path_buf()],
            installed_programs: vec![InstalledProgram {
                display_name: "Registry-only".into(),
                publisher: Some("Publisher".into()),
                install_location: Some(r"C:\Other".into()),
                display_icon: Some(r"C:\Other\icon.exe,0".into()),
            }],
            ..Default::default()
        };
        let no_sources = scan(
            &ScanRoots { start_menu_dirs: Vec::new(), ..roots.clone() },
            ScanOptions { start_menu: false, installed_program_metadata: true, packaged: false },
        );
        assert!(no_sources.is_empty());
    }

    #[test]
    fn packaged_apps_use_aumids_and_can_be_disabled() {
        let roots = ScanRoots {
            packaged_apps: vec![PackagedApplication {
                family_name: "Microsoft.WindowsCalculator_8wekyb3d8bbwe".into(),
                app_id: "App".into(),
                name: "Calculator".into(),
                install_dir: Some(r"C:\Program Files\WindowsApps\Calculator".into()),
            }],
            ..Default::default()
        };
        let apps = scan(&roots, ScanOptions { start_menu: false, installed_program_metadata: false, packaged: true });
        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].launch.aumid.as_deref(), Some("Microsoft.WindowsCalculator_8wekyb3d8bbwe!App"));
        assert_eq!(scan(&roots, ScanOptions { start_menu: false, installed_program_metadata: false, packaged: false }).len(), 0);
    }
}
