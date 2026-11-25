use heronite_data::apps::AppPrefs;

use crate::{PackagedApplication, ScanRoots};

mod uninstall;

pub fn this_pc() -> anyhow::Result<ScanRoots> {
    let start_menu_dirs = heronite_platform::known_folders::programs()?;
    let installed_programs = uninstall::installed_programs();
    let packaged_apps = heronite_platform::packages::installed()
        .into_iter()
        .flat_map(|package| {
            let family_name = package.family_name();
            let name = heronite_platform::packages::resolve(&package.full_name, &package.display_name)
                .unwrap_or_else(|| package.name().to_string());
            package.apps.into_iter().map(move |app_id| PackagedApplication {
                family_name: family_name.clone(),
                app_id,
                name: name.clone(),
                install_dir: Some(package.root.to_string_lossy().into_owned()),
            })
        })
        .collect();
    Ok(ScanRoots { start_menu_dirs, installed_programs, packaged_apps })
}

pub fn dump(roots: &ScanRoots, prefs: &AppPrefs) -> String {
    let candidates = crate::scan(roots, prefs.into());
    serde_dump(&candidates)
}

fn serde_dump(candidates: &[heronite_data::apps::AppCandidate]) -> String {
    serde_json::to_string_pretty(candidates).unwrap_or_else(|error| format!("{candidates:#?}\nSerialization failed: {error}"))
}
