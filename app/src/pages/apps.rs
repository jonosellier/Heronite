//! The Apps page of the Stores (Windows): discovers non-game Windows
//! applications from the Start Menu and installed packages (`services::apps_library`)
//! and keeps them in the Collection under the "Apps" store.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::apps::AppCandidate;
use heronite_i18n::{tr, trn};

use super::commands::{CommandCtx, PageCommand, PaletteCommands};
use super::methods;
use super::{empty_row, list_item, stores, Page, PageCtx, PageEvent};
use crate::console::kit::{self, u};
use crate::console::list::{Outcome, Row};
use crate::console::{hint, Btn, Hint, Pad, PadTarget};
use crate::icons::Icon;
use crate::services::apps_library::{AppSource, AppsLibrary, AppsLibraryEvent};
use crate::theme;
use crate::ui::controls::{button_sm, caption, card, icon_button_sm, toggle_row, with_sub_rows, ButtonKind};
use crate::ui::icon;

pub struct AppsPage {
    svc: Entity<AppsLibrary>,
    console_list: crate::console::list::ListNav,
    _subs: Vec<Subscription>,
}

impl EventEmitter<PageEvent> for AppsPage {}

const KEYWORDS: &str = "apps applications programs tools start menu windows loja aplicativos programas";

fn read_at(at: &str) -> Option<String> {
    let t = chrono::DateTime::parse_from_rfc3339(at).ok()?.with_timezone(&chrono::Local);
    Some(tr!("Last read {when}", when = t.format("%Y-%m-%d %H:%M")))
}

impl AppsPage {
    pub fn new(_ctx: PageCtx, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        let svc = crate::services::apps_library(cx);
        stores::watch(cx);
        let subs = vec![
            cx.observe(&svc, |_, _, cx| cx.notify()),
            cx.subscribe(&svc, |_, _, ev: &AppsLibraryEvent, cx| {
                let changed = match ev {
                    AppsLibraryEvent::Synced(report) => report.added + report.updated + report.removed > 0,
                    AppsLibraryEvent::PrefsChanged => true,
                };
                if changed {
                    cx.emit(PageEvent::LibraryChanged);
                }
            }),
        ];
        Self { svc, console_list: Default::default(), _subs: subs }
    }

    /// Refreshes the list when the page is shown again.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.svc.update(cx, |s, cx| s.sync_quietly(cx));
    }

    fn run_action(&mut self, id: &str, cx: &mut Context<Self>) {
        match id {
            "scan" => self.svc.update(cx, |s, cx| s.sync(cx)),
            "toggle-enabled" => {
                let enabled = self.svc.read(cx).prefs.enabled;
                self.svc.update(cx, |s, cx| s.set_enabled(!enabled, cx));
            }
            "toggle-main" => {
                let show = self.svc.read(cx).prefs.show_in_main_library;
                self.svc.update(cx, |s, cx| s.set_show_in_main_library(!show, cx));
            }
            "toggle-badges" => {
                let show = self.svc.read(cx).prefs.show_badges;
                self.svc.update(cx, |s, cx| s.set_show_badges(!show, cx));
            }
            "toggle-start-menu" => {
                let on = self.svc.read(cx).prefs.import_start_menu;
                self.svc.update(cx, |s, cx| s.set_source(AppSource::StartMenu, !on, cx));
            }
            "toggle-installed-programs" => {
                let on = self.svc.read(cx).prefs.import_installed_programs;
                self.svc.update(cx, |s, cx| s.set_source(AppSource::InstalledPrograms, !on, cx));
            }
            "toggle-packaged" => {
                let on = self.svc.read(cx).prefs.import_packaged;
                self.svc.update(cx, |s, cx| s.set_source(AppSource::Packaged, !on, cx));
            }
            _ => {}
        }
        cx.notify();
    }

    fn show_app(&mut self, candidate: &AppCandidate, cx: &mut Context<Self>) {
        cx.emit(PageEvent::ShowGame(format!("sideload_{}", candidate.id)));
    }

    fn summary_line(&self, cx: &App) -> String {
        let s = self.svc.read(cx);
        if !s.prefs.enabled {
            return tr!("Off \u{b7} Turn on below to find Windows apps and tools on this PC").to_string();
        }
        let mut parts = vec![trn!(s.candidates.len(), "{n} app found", "{n} apps found")];
        if let Some(at) = s.prefs.last.as_ref().and_then(|l| read_at(&l.at)) {
            parts.push(at);
        }
        parts.join("  \u{b7}  ")
    }

    fn result_line(&self, cx: &App) -> Option<(String, bool)> {
        let s = self.svc.read(cx);
        if !s.prefs.enabled {
            return None;
        }
        match s.last.as_ref()? {
            Ok(r) if r.added > 0 => Some((trn!(r.added, "Added {n} app to the Collection.", "Added {n} apps to the Collection."), false)),
            Ok(r) if r.updated + r.removed > 0 => Some((tr!("Updated the Apps list.").to_string(), false)),
            Ok(_) => Some((tr!("The Apps list is up to date.").to_string(), false)),
            Err(e) => Some((e.clone(), true)),
        }
    }

    fn render_settings(&self, cx: &mut Context<Self>) -> Div {
        let (enabled, show_main, show_badges, start_menu, installed_programs, packaged) = {
            let prefs = &self.svc.read(cx).prefs;
            (
                prefs.enabled,
                prefs.show_in_main_library,
                prefs.show_badges,
                prefs.import_start_menu,
                prefs.import_installed_programs,
                prefs.import_packaged,
            )
        };

        let enable_row = toggle_row(
            "apps-enabled",
            tr!("Find Windows apps on this PC"),
            Some(tr!("Scan Start Menu shortcuts and installed packages, keeping games out.")),
            enabled,
        )
        .on_click(cx.listener(|this, _, _, cx| this.run_action("toggle-enabled", cx)));

        let sub = if enabled {
            vec![
                toggle_row(
                    "apps-show-main",
                    tr!("Show in main Collection"),
                    Some(tr!("When off, apps are hidden until you filter by the Apps store.")),
                    show_main,
                )
                .on_click(cx.listener(|this, _, _, cx| this.run_action("toggle-main", cx)))
                .into_any_element(),
                toggle_row(
                    "apps-show-badges",
                    tr!("Show Apps badge on covers"),
                    Some(tr!("Display the computer icon badge on imported app covers.")),
                    show_badges,
                )
                .on_click(cx.listener(|this, _, _, cx| this.run_action("toggle-badges", cx)))
                .into_any_element(),
            ]
        } else {
            Vec::new()
        };

        let mut out = div().child(card([with_sub_rows(enable_row.into_any_element(), sub)]));
        if enabled {
            let sources = card([
                toggle_row(
                    "apps-src-start-menu",
                    tr!("Start Menu shortcuts"),
                    Some(tr!("Read .lnk and .url shortcuts from Start Menu Programs folders.")),
                    start_menu,
                )
                .on_click(cx.listener(|this, _, _, cx| this.run_action("toggle-start-menu", cx)))
                .into_any_element(),
                toggle_row(
                    "apps-src-uninstall",
                    tr!("Installed program details"),
                    Some(tr!("Use Windows uninstall registry entries for publishers, folders and icons.")),
                    installed_programs,
                )
                .on_click(cx.listener(|this, _, _, cx| this.run_action("toggle-installed-programs", cx)))
                .into_any_element(),
                toggle_row(
                    "apps-src-packaged",
                    tr!("Microsoft Store & packaged apps"),
                    Some(tr!("Include MSIX and UWP apps installed for this Windows user.")),
                    packaged,
                )
                .on_click(cx.listener(|this, _, _, cx| this.run_action("toggle-packaged", cx)))
                .into_any_element(),
            ]);
            out = out.child(caption(tr!("Sources"))).child(sources);
        }
        out
    }

    fn candidate_detail(candidate: &AppCandidate) -> String {
        if let Some(publisher) = candidate.publisher.as_deref().filter(|p| !p.trim().is_empty()) {
            return publisher.to_string();
        }
        if let Some(dir) = candidate.install_dir.as_deref().filter(|d| !d.trim().is_empty()) {
            return dir.to_string();
        }
        if !candidate.launch.target.trim().is_empty() {
            return candidate.launch.target.clone();
        }
        candidate.launch.aumid.clone().unwrap_or_default()
    }

    fn render_app_row(&self, candidate: &AppCandidate, cx: &mut Context<Self>) -> AnyElement {
        let content = div()
            .flex()
            .flex_col()
            .gap(px(2.))
            .min_w_0()
            .child(div().text_size(px(14.)).text_color(theme::text_primary()).truncate().child(candidate.name.clone()))
            .child(div().text_size(px(12.)).text_color(theme::text_dim()).truncate().child(Self::candidate_detail(candidate)));
        let shown = candidate.clone();
        let hide_id = candidate.id.clone();
        let actions = div()
            .flex()
            .items_center()
            .gap(px(6.))
            .child(
                button_sm(SharedString::from(format!("apps-show-{}", candidate.id)), ButtonKind::Glass, tr!("Show"))
                    .on_click(cx.listener(move |this, _, _, cx| this.show_app(&shown, cx))),
            )
            .child(
                button_sm(SharedString::from(format!("apps-hide-{}", candidate.id)), ButtonKind::Ghost, tr!("Hide")).on_click(
                    cx.listener(move |this, _, _, cx| {
                        let id = hide_id.clone();
                        this.svc.update(cx, |s, cx| s.set_hidden(id, true, cx));
                    }),
                ),
            );
        list_item(Some(icon(Icon::Computer, px(22.), theme::text_muted()).into_any_element()), content, actions).into_any_element()
    }

    fn render_apps_list(&self, cx: &mut Context<Self>) -> Div {
        let (enabled, candidates, skipped_titles, hidden_ids, hidden_candidates) = {
            let s = self.svc.read(cx);
            (
                s.prefs.enabled,
                s.candidates.clone(),
                s.skipped_titles.clone(),
                s.prefs.hidden_ids.clone(),
                s.hidden_candidates.clone(),
            )
        };
        let rows: Vec<AnyElement> = if !enabled {
            vec![empty_row(tr!("Turn on \"Find Windows apps on this PC\" above to scan for non-game apps.")).into_any_element()]
        } else if candidates.is_empty() {
            vec![empty_row(tr!("No apps found yet. Click Scan now or check your enabled sources.")).into_any_element()]
        } else {
            candidates.iter().map(|c| self.render_app_row(c, cx)).collect()
        };
        let mut out = div().child(caption(tr!("Imported apps"))).child(card(rows));

        if enabled && !skipped_titles.is_empty() {
            let skipped_rows: Vec<AnyElement> = skipped_titles
                .iter()
                .map(|c| {
                    let id = c.id.clone();
                    let content = div()
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .min_w_0()
                        .child(div().text_size(px(14.)).text_color(theme::text_primary()).truncate().child(c.name.clone()))
                        .child(
                            div()
                                .text_size(px(12.))
                                .text_color(theme::text_dim())
                                .truncate()
                                .child(tr!("Skipped because a game in your Collection has the same title")),
                        );
                    let allow = button_sm(SharedString::from(format!("apps-allow-title-{}", c.id)), ButtonKind::Glass, tr!("Show anyway"))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let id = id.clone();
                            this.svc.update(cx, |svc, cx| svc.set_show_title_match(id, true, cx));
                        }));
                    list_item(Some(icon(Icon::Computer, px(22.), theme::text_muted()).into_any_element()), content, allow).into_any_element()
                })
                .collect();
            out = out.child(caption(tr!("Matched games by title"))).child(card(skipped_rows));
        }

        if enabled && !hidden_ids.is_empty() {
            let hidden_rows: Vec<AnyElement> = hidden_ids
                .iter()
                .map(|id| {
                    let candidate = hidden_candidates.iter().find(|c| c.id == *id);
                    let title = candidate.map(|c| c.name.clone()).unwrap_or_else(|| id.clone());
                    let detail = candidate.map(Self::candidate_detail).unwrap_or_default();
                    let restore_id = id.clone();
                    let content = div()
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .min_w_0()
                        .child(div().text_size(px(14.)).text_color(theme::text_primary()).truncate().child(title))
                        .when(!detail.is_empty(), |d| {
                            d.child(div().text_size(px(12.)).text_color(theme::text_dim()).truncate().child(detail))
                        });
                    let restore =
                        button_sm(SharedString::from(format!("apps-restore-{id}")), ButtonKind::Glass, tr!("Restore")).on_click(cx.listener(
                            move |this, _, _, cx| {
                                let id = restore_id.clone();
                                this.svc.update(cx, |svc, cx| svc.set_hidden(id, false, cx));
                            },
                        ));
                    list_item(Some(icon(Icon::Computer, px(22.), theme::text_dim()).into_any_element()), content, restore)
                        .into_any_element()
                })
                .collect();
            out = out.child(caption(tr!("Hidden apps"))).child(card(hidden_rows));
        }

        out
    }

    fn console_rows(&self, cx: &App) -> Vec<Row> {
        let s = self.svc.read(cx);
        let prefs = &s.prefs;
        let mut rows = vec![
            Row::header(tr!("Apps")),
            Row::toggle("toggle-enabled", tr!("Find Windows apps on this PC"), prefs.enabled)
                .desc(self.summary_line(cx))
                .icon(Icon::Computer),
        ];
        if prefs.enabled {
            rows.push(
                Row::action("scan", if s.syncing { tr!("Scanning\u{2026}") } else { tr!("Scan now") })
                    .icon(Icon::Refresh)
                    .enabled(!s.syncing),
            );
            rows.push(Row::toggle("toggle-main", tr!("Show in main Collection"), prefs.show_in_main_library));
            rows.push(Row::toggle("toggle-badges", tr!("Show Apps badge on covers"), prefs.show_badges));
            rows.push(Row::header(tr!("Sources")));
            rows.push(Row::toggle("toggle-start-menu", tr!("Start Menu shortcuts"), prefs.import_start_menu));
            rows.push(Row::toggle("toggle-installed-programs", tr!("Installed program details"), prefs.import_installed_programs));
            rows.push(Row::toggle("toggle-packaged", tr!("Microsoft Store & packaged apps"), prefs.import_packaged));
        }
        rows
    }

    fn render_console(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let rows = self.console_rows(cx);
        let list = self.console_list.render_cards("apps-console", &rows, true);
        div()
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .h(u(76.))
                    .px(u(46.))
                    .gap(u(36.))
                    .child(kit::wordmark(&stores::title().to_uppercase()))
                    .child(stores::console_tabs(Page::Apps))
                    .child(div().flex_1())
                    .child(kit::clock()),
            )
            .child(div().flex().flex_col().flex_1().min_h_0().px(u(46.)).pb(u(16.)).child(list))
            .into_any_element()
    }
}

impl Render for AppsPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if crate::console::active() {
            return self.render_console(cx);
        }
        let s = self.svc.read(cx);
        let scan = icon_button_sm(
            "apps-scan",
            ButtonKind::Glass,
            Icon::Refresh,
            if s.syncing { tr!("Scanning\u{2026}") } else { tr!("Scan now") },
            s.prefs.enabled && !s.syncing,
            cx.listener(|this, _, _, cx| this.run_action("scan", cx)),
        );
        let strip = methods::library_strip(self.summary_line(cx), self.result_line(cx), scan);
        let body = div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(strip)
            .child(self.render_settings(cx))
            .child(self.render_apps_list(cx));
        div()
            .size_full()
            .relative()
            .child(stores::sheet(
                "apps-page",
                Page::Apps,
                tr!("Non-game Windows apps and tools on this PC, kept apart from your games unless you choose to show them."),
                cx,
                body,
            ))
            .into_any_element()
    }
}

impl PadTarget for AppsPage {
    fn on_pad(&mut self, pad: Pad, _window: &mut Window, cx: &mut Context<Self>) -> bool {
        if let Some(next) = stores::switch(Page::Apps, pad) {
            cx.emit(PageEvent::Navigate(next));
            return true;
        }
        let rows = self.console_rows(cx);
        let used = match self.console_list.on_pad(pad, &rows) {
            Outcome::Activate(id) | Outcome::Changed(id, _) => {
                self.run_action(id.as_ref(), cx);
                true
            }
            Outcome::Ignored => false,
            _ => true,
        };
        cx.notify();
        used
    }

    fn hints(&self, _cx: &App) -> Vec<Hint> {
        vec![hint(Btn::A, tr!("Select"))]
    }
}

impl PaletteCommands for AppsPage {
    fn palette_commands(_page: Option<&Self>, _ctx: &CommandCtx, cx: &App) -> Vec<PageCommand> {
        let state = crate::services::apps_library_started(cx).map(|s| {
            let s = s.read(cx);
            (s.prefs.enabled, s.syncing)
        });
        let (enabled, syncing) = state.unwrap_or((false, false));
        vec![
            PageCommand::new(
                "scan",
                tr!("Scan Windows apps"),
                tr!("Find non-game Windows apps and tools installed on this PC"),
                Icon::Refresh,
            )
            .keywords(KEYWORDS)
            .disabled((!enabled).then(|| tr!("Apps disabled")).or_else(|| syncing.then(|| tr!("Scanning\u{2026}"))))
            .stay(),
            PageCommand::new(
                "toggle-enabled",
                if enabled { tr!("Turn off Windows apps") } else { tr!("Turn on Windows apps") },
                tr!("Scan Start Menu shortcuts and installed packages for non-game apps"),
                Icon::Computer,
            )
            .keywords(KEYWORDS)
            .stay(),
        ]
    }

    fn run_palette_command(&mut self, id: &str, _window: &mut Window, cx: &mut Context<Self>) {
        self.run_action(id, cx);
    }

    fn page_modal(&self) -> bool {
        false
    }
}
