//! Imported Windows applications from the Start Menu and installed packages.

use std::collections::BTreeSet;
use std::time::Duration;

use gpui_kit::*;
use heronite_apps::collection::{self, SyncReport};
use heronite_data::apps::{self, AppCandidate, AppPrefs, Classification, GameMatch, GameRef};
use heronite_data::{CollectionGame, DataPaths, Library};

const FIRST_DELAY: Duration = Duration::from_secs(8);

pub enum AppsLibraryEvent {
    Synced(SyncReport),
    PrefsChanged,
}

pub struct AppsLibrary {
    paths: DataPaths,
    pub syncing: bool,
    pub last: Option<Result<SyncReport, String>>,
    pub prefs: AppPrefs,
    pub candidates: Vec<AppCandidate>,
    pub hidden_candidates: Vec<AppCandidate>,
    pub skipped_titles: Vec<AppCandidate>,
    raw_candidates: Vec<AppCandidate>,
}

impl EventEmitter<AppsLibraryEvent> for AppsLibrary {}

struct ScanResult {
    report: SyncReport,
    candidates: Vec<AppCandidate>,
    hidden_candidates: Vec<AppCandidate>,
    skipped_titles: Vec<AppCandidate>,
    raw_candidates: Vec<AppCandidate>,
}

fn partition_candidates(paths: &DataPaths, prefs: &AppPrefs, raw: Vec<AppCandidate>) -> Result<ScanResult, String> {
    let library = Library::load_with(paths, true).map_err(|error| format!("{error:#}"))?;
    let games = game_refs(paths, &library.games);
    let mut candidates = Vec::new();
    let mut hidden_candidates = Vec::new();
    let mut skipped_titles = Vec::new();
    for candidate in &raw {
        match apps::classify(candidate, &games, &prefs.force_includes, &prefs.show_title_matches) {
            Classification::App => {
                if prefs.hidden_ids.contains(&candidate.id) || apps::is_hidden(candidate, prefs) {
                    hidden_candidates.push(candidate.clone());
                } else {
                    candidates.push(candidate.clone());
                }
            }
            Classification::Game(GameMatch::Title) => skipped_titles.push(candidate.clone()),
            Classification::Game(_) => {}
        }
    }
    let report = collection::sync(paths, &raw, &games).map_err(|error| format!("{error:#}"))?;
    Ok(ScanResult { report, candidates, hidden_candidates, skipped_titles, raw_candidates: raw })
}

fn sync_blocking(paths: &DataPaths) -> Result<(AppPrefs, ScanResult), String> {
    let prefs = AppPrefs::load(paths);
    if !prefs.enabled {
        let report = collection::sync(paths, &[], &[]).map_err(|error| format!("{error:#}"))?;
        let prefs = AppPrefs::load(paths);
        return Ok((
            prefs,
            ScanResult {
                report,
                candidates: Vec::new(),
                hidden_candidates: Vec::new(),
                skipped_titles: Vec::new(),
                raw_candidates: Vec::new(),
            },
        ));
    }
    let roots = heronite_apps::this_pc().map_err(|error| format!("{error:#}"))?;
    let raw = heronite_apps::scan(&roots, (&prefs).into());
    if crate::env::value("HERONITE_DEBUG_APPS").as_deref() == Some("dump") {
        log::info!("Apps scan candidates:\n{}", serde_json::to_string_pretty(&raw).unwrap_or_else(|error| error.to_string()));
    }
    let result = partition_candidates(paths, &prefs, raw)?;
    let prefs = AppPrefs::load(paths);
    Ok((prefs, result))
}

fn game_refs(paths: &DataPaths, games: &[std::sync::Arc<CollectionGame>]) -> Vec<GameRef> {
    let mut references: Vec<GameRef> = games
        .iter()
        .filter(|game| !game.is_app())
        .map(|game| {
            let meta = game.meta.as_deref();
            let mut store_ids = BTreeSet::new();
            store_ids.insert(game.app_name.clone());
            if let Some(id) = game.steam_app_id.as_deref().filter(|id| !id.is_empty()) {
                store_ids.insert(id.to_string());
            }
            if let Some(id) = meta.and_then(|m| m.extra.get(heronite_data::xbox::STORE_ID_KEY)).and_then(serde_json::Value::as_str) {
                store_ids.insert(id.to_string());
            }
            if let Some(id) = meta.and_then(|m| m.extra.get(heronite_data::xbox::TITLE_ID_KEY)).and_then(serde_json::Value::as_str) {
                store_ids.insert(id.to_string());
            }
            GameRef {
                store: meta.map_or_else(|| game.source_label.to_ascii_lowercase(), |m| m.source.clone()),
                title: game.title.clone(),
                install_dir: game.details.install_path.clone().or_else(|| game.game.install.install_path.clone()),
                exe: meta.and_then(|m| m.remapped_executable.clone()).or_else(|| game.game.install.executable.clone()),
                store_ids,
                aumid: meta.filter(|m| m.launch_kind == heronite_data::xbox::LAUNCH_KIND).and_then(|m| m.launcher_args.clone()),
            }
        })
        .collect();
    let xbox = heronite_data::xbox::XboxPrefs::load(paths);
    references.extend(xbox.removed.into_iter().chain(xbox.removed_titles).map(|id| GameRef {
        store: "xbox".into(),
        title: String::new(),
        install_dir: None,
        exe: None,
        store_ids: [id].into_iter().collect(),
        aumid: None,
    }));
    let clients = heronite_data::clients::ClientPrefs::load(paths);
    references.extend(clients.removed.into_iter().flat_map(|(client, ids)| {
        ids.into_iter().map(move |id| GameRef {
            store: client.id().into(),
            title: String::new(),
            install_dir: None,
            exe: None,
            store_ids: [id].into_iter().collect(),
            aumid: None,
        })
    }));
    references.extend(clients.removed_owned.into_iter().flat_map(|(client, ids)| {
        ids.into_iter().map(move |id| GameRef {
            store: client.id().into(),
            title: String::new(),
            install_dir: None,
            exe: None,
            store_ids: [id].into_iter().collect(),
            aumid: None,
        })
    }));
    references
}

impl AppsLibrary {
    pub fn new(paths: DataPaths, cx: &mut Context<Self>) -> Self {
        let prefs = AppPrefs::load(&paths);
        let this = Self {
            paths,
            syncing: false,
            last: None,
            prefs,
            candidates: Vec::new(),
            hidden_candidates: Vec::new(),
            skipped_titles: Vec::new(),
            raw_candidates: Vec::new(),
        };
        if heronite_platform::supported(heronite_platform::Feature::AppsStore)
            && this.prefs.enabled
            && !crate::env::flag("HERONITE_DISABLE_IDLE_JOBS")
        {
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(FIRST_DELAY).await;
                this.update(cx, |this, cx| this.run(false, cx)).ok();
            })
            .detach();
        }
        this
    }

    pub fn sync(&mut self, cx: &mut Context<Self>) {
        self.run(true, cx);
    }

    pub fn sync_quietly(&mut self, cx: &mut Context<Self>) {
        if self.prefs.enabled {
            self.run(false, cx);
        }
    }

    pub fn set_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.update_prefs(true, true, move |prefs| prefs.enabled = enabled, cx);
    }

    pub fn set_show_in_main_library(&mut self, show: bool, cx: &mut Context<Self>) {
        self.update_prefs(false, false, move |prefs| prefs.show_in_main_library = show, cx);
    }

    pub fn set_show_badges(&mut self, show: bool, cx: &mut Context<Self>) {
        self.update_prefs(false, false, move |prefs| prefs.show_badges = show, cx);
    }

    pub fn set_source(&mut self, source: AppSource, enabled: bool, cx: &mut Context<Self>) {
        self.update_prefs(
            true,
            true,
            move |prefs| match source {
                AppSource::StartMenu => prefs.import_start_menu = enabled,
                AppSource::InstalledPrograms => prefs.import_installed_programs = enabled,
                AppSource::Packaged => prefs.import_packaged = enabled,
            },
            cx,
        );
    }

    pub fn set_hidden(&mut self, id: String, hidden: bool, cx: &mut Context<Self>) {
        self.update_prefs(
            false,
            true,
            move |prefs| {
                if hidden {
                    prefs.hidden_ids.insert(id.clone());
                } else {
                    prefs.hidden_ids.remove(&id);
                }
            },
            cx,
        );
    }

    pub fn set_show_title_match(&mut self, id: String, show: bool, cx: &mut Context<Self>) {
        self.update_prefs(
            false,
            true,
            move |prefs| {
                if show {
                    prefs.show_title_matches.insert(id.clone());
                } else {
                    prefs.show_title_matches.remove(&id);
                }
            },
            cx,
        );
    }

    pub fn note_hidden(&mut self, id: &str, cx: &mut Context<Self>) {
        let id = id.to_string();
        self.update_prefs(
            false,
            true,
            move |prefs| {
                prefs.hidden_ids.insert(id.clone());
            },
            cx,
        );
    }

    fn update_prefs(&mut self, rescan: bool, resync_collection: bool, mutate: impl Fn(&mut AppPrefs) + Send + 'static, cx: &mut Context<Self>) {
        mutate(&mut self.prefs);
        let paths = self.paths.clone();
        let cached_raw = (!rescan && resync_collection && !self.raw_candidates.is_empty()).then(|| self.raw_candidates.clone());
        cx.spawn(async move |this, cx| {
            let outcome: Result<(AppPrefs, Option<ScanResult>), String> = cx
                .background_spawn(async move {
                    heronite_data::json_store::update_if_changed::<AppPrefs, _>(&paths.apps(), |prefs| {
                        mutate(prefs);
                        Ok(())
                    })
                    .map_err(|error| format!("{error:#}"))?;
                    let prefs = AppPrefs::load(&paths);
                    if !resync_collection {
                        return Ok((prefs, None));
                    }
                    if !prefs.enabled {
                        let report = collection::sync(&paths, &[], &[]).map_err(|error| format!("{error:#}"))?;
                        return Ok((
                            AppPrefs::load(&paths),
                            Some(ScanResult {
                                report,
                                candidates: Vec::new(),
                                hidden_candidates: Vec::new(),
                                skipped_titles: Vec::new(),
                                raw_candidates: Vec::new(),
                            }),
                        ));
                    }
                    if let Some(raw) = cached_raw {
                        let scan = partition_candidates(&paths, &prefs, raw)?;
                        return Ok((AppPrefs::load(&paths), Some(scan)));
                    }
                    let (_, scan) = sync_blocking(&paths)?;
                    Ok((AppPrefs::load(&paths), Some(scan)))
                })
                .await;
            this.update(cx, |this, cx| {
                match outcome {
                    Ok((prefs, Some(scan))) => {
                        this.prefs = prefs;
                        this.candidates = scan.candidates;
                        this.hidden_candidates = scan.hidden_candidates;
                        this.skipped_titles = scan.skipped_titles;
                        this.raw_candidates = scan.raw_candidates;
                        cx.emit(AppsLibraryEvent::Synced(scan.report));
                        cx.emit(AppsLibraryEvent::PrefsChanged);
                    }
                    Ok((prefs, None)) => {
                        this.prefs = prefs;
                        cx.emit(AppsLibraryEvent::PrefsChanged);
                    }
                    Err(error) => log::warn!("Apps preferences: {error}"),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn run(&mut self, asked: bool, cx: &mut Context<Self>) {
        if self.syncing {
            return;
        }
        self.syncing = true;
        let paths = self.paths.clone();
        cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(async move { sync_blocking(&paths) }).await;
            this.update(cx, |this, cx| {
                this.syncing = false;
                let outcome = match result {
                    Ok((prefs, scan)) => {
                        this.prefs = prefs;
                        this.candidates = scan.candidates;
                        this.hidden_candidates = scan.hidden_candidates;
                        this.skipped_titles = scan.skipped_titles;
                        this.raw_candidates = scan.raw_candidates;
                        cx.emit(AppsLibraryEvent::Synced(scan.report));
                        Ok(scan.report)
                    }
                    Err(error) => {
                        log::warn!("Apps scan: {error}");
                        Err(error)
                    }
                };
                if asked {
                    this.last = Some(outcome);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppSource {
    StartMenu,
    InstalledPrograms,
    Packaged,
}
