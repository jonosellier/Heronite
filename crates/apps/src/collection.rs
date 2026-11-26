//! Sideload records for the Apps store.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::Result;
use heronite_data::apps::{self, AppCandidate, AppPrefs, Classification};
use heronite_data::json_store;
use heronite_data::models::{GameInfo, InstallInfo, LocalGameMeta, LocalMetaFile};
use heronite_data::DataPaths;
use serde_json::{json, Value};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SyncReport {
    pub added: usize,
    pub updated: usize,
    pub removed: usize,
    pub skipped_games: usize,
}

pub fn sync(paths: &DataPaths, candidates: &[AppCandidate], games: &[apps::GameRef]) -> Result<SyncReport> {
    let mut prefs = AppPrefs::load(paths);
    let mut report = SyncReport::default();
    
    // Auto-hide new apps by default: add any app not already in hidden_ids or force_includes to hidden_ids
    if prefs.enabled {
        for candidate in candidates {
            let is_app = matches!(apps::classify(candidate, games, &prefs.force_includes, &prefs.show_title_matches), Classification::App);
            if is_app && !prefs.hidden_ids.contains(&candidate.id) && !prefs.force_includes.contains(&candidate.id) {
                prefs.hidden_ids.insert(candidate.id.clone());
            }
        }
        // Save prefs if any new apps were auto-hidden
        prefs.save(paths).ok();
    }
    
    let importable: Vec<&AppCandidate> = if prefs.enabled {
        candidates
            .iter()
            .filter(|candidate| {
                let is_app = matches!(apps::classify(candidate, games, &prefs.force_includes, &prefs.show_title_matches), Classification::App);
                if !is_app {
                    report.skipped_games += 1;
                    return false;
                }
                !prefs.hidden_ids.contains(&candidate.id) && !apps::is_hidden(candidate, &prefs)
            })
            .collect()
    } else {
        Vec::new()
    };
    let wanted: BTreeSet<String> = importable.iter().map(|candidate| candidate.id.clone()).collect();
    let mut metas: LocalMetaFile = json_store::read(&paths.local_meta())?;
    let old: BTreeSet<String> = metas
        .games
        .values()
        .filter(|meta| meta.source == apps::SOURCE && meta.launch_kind == apps::LAUNCH_KIND)
        .map(|meta| meta.app_name.clone())
        .collect();
    for app_name in old.difference(&wanted) {
        metas.games.shift_remove(app_name);
        report.removed += 1;
    }
    for candidate in &importable {
        let is_new = !old.contains(&candidate.id);
        let meta = metas.games.entry(candidate.id.clone()).or_insert_with(|| LocalGameMeta {
            app_name: candidate.id.clone(),
            runner: "sideload".into(),
            source: apps::SOURCE.into(),
            launch_kind: apps::LAUNCH_KIND.into(),
            ..Default::default()
        });
        meta.app_name = candidate.id.clone();
        meta.runner = "sideload".into();
        meta.source = apps::SOURCE.into();
        meta.launch_kind = apps::LAUNCH_KIND.into();
        meta.title = candidate.name.clone();
        meta.extra.insert(apps::CANDIDATE_KEY.into(), serde_json::to_value(*candidate)?);
        if is_new {
            report.added += 1;
        }
    }

    if !old.is_empty() || !wanted.is_empty() {
        json_store::update_if_changed::<LocalMetaFile, _>(&paths.local_meta(), |file| {
            *file = metas;
            Ok(())
        })?;
    }

    let mut expected = BTreeMap::new();
    for candidate in importable {
        expected.insert(candidate.id.clone(), game_entry(candidate)?);
    }
    let previous_entries = if paths.sideload_library().exists() { json_store::read_value(&paths.sideload_library())? } else { Value::Null };
    let previous: BTreeSet<String> = previous_entries
        .get("games")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            let app_name = entry.get("app_name")?.as_str()?;
            old.contains(app_name).then(|| app_name.to_string())
        })
        .collect();
    let expected_names: BTreeSet<String> = expected.keys().cloned().collect();
    report.removed = report.removed.max(previous.difference(&expected_names).count());
    if !old.is_empty() || !expected.is_empty() {
        json_store::update_value_if_changed(&paths.sideload_library(), |document| {
            if !document.is_object() {
                *document = json!({});
            }
            let object = document.as_object_mut().expect("object");
            let games = object.entry("games").or_insert_with(|| json!([]));
            if !games.is_array() {
                *games = json!([]);
            }
            let list = games.as_array_mut().expect("array");
            let mut found = BTreeSet::new();
            list.retain_mut(|entry| {
                let Some(app_name) = entry.get("app_name").and_then(Value::as_str).map(str::to_string) else {
                    return true;
                };
                if old.contains(&app_name) && !expected.contains_key(&app_name) {
                    return false;
                }
                if let Some(fresh) = expected.get(&app_name) {
                    found.insert(app_name);
                    if *entry != *fresh {
                        *entry = fresh.clone();
                        report.updated += 1;
                    }
                }
                true
            });
            for (app_name, entry) in &expected {
                if !found.contains(app_name) {
                    list.push(entry.clone());
                }
            }
            Ok(())
        })?;
    }
    if prefs.enabled {
        let app_count = expected.len();
        let touched = report.added > 0 || report.updated > 0 || report.removed > 0;
        json_store::update_if_changed::<AppPrefs, _>(&paths.apps(), |current| {
            if touched || current.last.as_ref().map(|last| last.apps) != Some(app_count) {
                current.last = Some(apps::LastSync {
                    at: chrono_now(),
                    apps: app_count,
                });
            }
            Ok(())
        })?;
    }
    Ok(report)
}

fn chrono_now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Format as RFC 3339 UTC without adding a chrono dependency to `heronite-apps`.
    // Wait: `apps` doesn't depend on `chrono`, so let's store unix seconds or check if `chrono` is in workspace.
    // Even simpler: format UTC seconds via civil-from-days!
    let days = (secs / 86_400) as i64;
    let sod = secs % 86_400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = y + i64::from(m <= 2);
    format!("{year:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", sod / 3600, (sod % 3600) / 60, sod % 60)
}

fn game_entry(candidate: &AppCandidate) -> Result<Value> {
    let executable = candidate
        .exe
        .clone()
        .or_else(|| candidate.launch.target.to_ascii_lowercase().ends_with(".exe").then(|| candidate.launch.target.clone()))
        .unwrap_or_default();
    let install_path =
        candidate.install_dir.clone().or_else(|| std::path::Path::new(&executable).parent().map(|p| p.to_string_lossy().into_owned()));
    let mut game = GameInfo {
        runner: "sideload".into(),
        app_name: candidate.id.clone(),
        title: candidate.name.clone(),
        is_installed: true,
        is_linux_native: Some(false),
        install: InstallInfo {
            executable: Some(executable),
            platform: Some("windows".into()),
            install_path: install_path.clone(),
            is_dlc: Some(false),
            ..Default::default()
        },
        ..Default::default()
    };
    if let Some(install_path) = install_path {
        game.folder_name = Some(install_path);
    }
    game.art_cover = None;
    game.art_square = None;
    game.rest.insert("canRunOffline".into(), Value::Bool(true));
    serde_json::to_value(game).map_err(Into::into)
}

pub fn set_hidden(paths: &DataPaths, id: &str, hidden: bool) -> Result<()> {
    json_store::update_if_changed::<AppPrefs, _>(&paths.apps(), |prefs| {
        if hidden {
            prefs.hidden_ids.insert(id.to_string());
        } else {
            prefs.hidden_ids.remove(id);
        }
        Ok(())
    })
}

pub fn set_allow_override(paths: &DataPaths, id: &str, allow: bool) -> Result<()> {
    json_store::update_if_changed::<AppPrefs, _>(&paths.apps(), |prefs| {
        if allow {
            prefs.allow_overrides.insert(id.to_string());
        } else {
            prefs.allow_overrides.remove(id);
        }
        Ok(())
    })
}

pub fn set_force_include(paths: &DataPaths, id: &str, include: bool) -> Result<()> {
    json_store::update_if_changed::<AppPrefs, _>(&paths.apps(), |prefs| {
        if include {
            prefs.force_includes.insert(id.to_string());
        } else {
            prefs.force_includes.remove(id);
        }
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use heronite_data::apps::{AppLaunch, AppOrigin, GameRef};

    #[test]
    fn sync_reconciles_apps_and_removes_entries_that_are_games() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        AppPrefs { enabled: true, ..Default::default() }.save(&paths).unwrap();
        let mut app =
            AppCandidate::new("Not a game", AppLaunch { target: r"C:\Apps\sample.exe".into(), ..Default::default() }, AppOrigin::StartMenu);
        app.exe = Some(r"C:\Apps\sample.exe".into());
        let first = sync(&paths, std::slice::from_ref(&app), &[]).unwrap();
        assert_eq!(first.added, 1);
        assert_eq!(json_store::read_value(&paths.sideload_library()).unwrap()["games"].as_array().unwrap().len(), 1);
        assert_eq!(json_store::read::<LocalMetaFile>(&paths.local_meta()).unwrap().games[&app.id].launch_kind, apps::LAUNCH_KIND);

        let matching_game = GameRef { exe: app.exe.clone(), title: "Not a game".into(), ..Default::default() };
        let second = sync(&paths, std::slice::from_ref(&app), &[matching_game]).unwrap();
        assert_eq!(second.removed, 1);
        assert!(json_store::read_value(&paths.sideload_library()).unwrap()["games"].as_array().unwrap().is_empty());
        assert!(!json_store::read::<LocalMetaFile>(&paths.local_meta()).unwrap().games.contains_key(&app.id));
    }

    #[test]
    fn disabling_or_blacklisting_apps_removes_their_collection_entries() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        AppPrefs { enabled: true, ..Default::default() }.save(&paths).unwrap();
        let mut app =
            AppCandidate::new("Useful Tool", AppLaunch { target: r"C:\Apps\tool.exe".into(), ..Default::default() }, AppOrigin::StartMenu);
        app.exe = Some(r"C:\Apps\tool.exe".into());
        let mut blacklisted =
            AppCandidate::new("Windows PowerShell", AppLaunch { target: r"C:\Apps\pwsh.exe".into(), ..Default::default() }, AppOrigin::StartMenu);
        blacklisted.exe = Some(r"C:\Apps\pwsh.exe".into());

        let report = sync(&paths, &[app.clone(), blacklisted], &[]).unwrap();
        assert_eq!(report.added, 1);
        assert_eq!(json_store::read_value(&paths.sideload_library()).unwrap()["games"].as_array().unwrap().len(), 1);

        AppPrefs { enabled: false, ..Default::default() }.save(&paths).unwrap();
        let disabled = sync(&paths, &[app], &[]).unwrap();
        assert_eq!(disabled.removed, 1);
        assert!(json_store::read_value(&paths.sideload_library()).unwrap()["games"].as_array().unwrap().is_empty());
        assert!(json_store::read::<LocalMetaFile>(&paths.local_meta()).unwrap().games.is_empty());
    }
}
