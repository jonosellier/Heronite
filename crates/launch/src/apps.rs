//! Launching imported Windows applications through the shell or executable.

use heronite_data::apps::{AppCandidate, AppLaunch};
use heronite_data::models::LocalGameMeta;
use heronite_i18n::tr;

use crate::args::split_args_for;
use crate::plan::{Command, Handoff, LaunchPlan, ProcessPlan};
use crate::probe::Probe;

pub(crate) fn build(probe: &dyn Probe, meta: &LocalGameMeta) -> LaunchPlan {
    if !cfg!(windows) {
        return LaunchPlan::Unavailable(tr!("Windows applications run on Windows only").into());
    }
    let launch = meta.extra.get(heronite_data::apps::CANDIDATE_KEY).and_then(|value| {
        serde_json::from_value::<AppCandidate>(value.clone())
            .map(|candidate| candidate.launch)
            .or_else(|_| serde_json::from_value::<AppLaunch>(value.clone()))
            .ok()
    });
    let Some(launch) = launch else {
        return LaunchPlan::Unavailable(tr!("This app has no launch method").into());
    };
    if let Some(shortcut) = launch.shortcut.filter(|path| !path.trim().is_empty() && probe.exists(path)) {
        return explorer_plan(vec![shortcut]);
    }
    if let Some(aumid) = launch.aumid.filter(|id| valid_aumid(id)) {
        return explorer_plan(vec![format!(r"shell:AppsFolder\{aumid}")]);
    }
    let target = launch.target.trim().to_string();
    if target.is_empty() || !probe.exists(&target) {
        return LaunchPlan::Unavailable(tr!("This app has no launch method").into());
    }
    if target.to_ascii_lowercase().ends_with(".exe") {
        let cwd = launch
            .working_dir
            .filter(|dir| !dir.trim().is_empty() && probe.exists(dir))
            .or_else(|| std::path::Path::new(&target).parent().map(|dir| dir.to_string_lossy().into_owned()).filter(|d| !d.is_empty()));
        return LaunchPlan::Native(ProcessPlan {
            command: Command { program: target, args: split_args_for(true, &launch.arguments), cwd, ..Default::default() },
            record_session: Some(false),
            notes: vec!["Windows starts the imported app directly".into()],
            ..Default::default()
        });
    }
    explorer_plan(vec![target])
}

fn explorer_plan(args: Vec<String>) -> LaunchPlan {
    LaunchPlan::Native(ProcessPlan {
        command: Command { program: "explorer.exe".into(), args, ..Default::default() },
        handoff: Some(Handoff { exit_ignored: true, ..Default::default() }),
        record_session: Some(false),
        notes: vec!["Windows starts the imported app through Explorer".into()],
        ..Default::default()
    })
}

fn valid_aumid(aumid: &str) -> bool {
    let ok = |part: &str| !part.is_empty() && part.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'));
    matches!(aumid.split_once('!'), Some((family, app)) if ok(family) && ok(app) && family.contains('_'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::LaunchPlan;
    use crate::probe::FakeProbe;
    use heronite_data::models::LocalGameMeta;

    fn meta(launch: AppLaunch) -> LocalGameMeta {
        LocalGameMeta {
            app_name: "app_example".into(),
            source: heronite_data::apps::SOURCE.into(),
            launch_kind: heronite_data::apps::LAUNCH_KIND.into(),
            extra: std::iter::once((heronite_data::apps::CANDIDATE_KEY.into(), serde_json::to_value(launch).unwrap())).collect(),
            ..Default::default()
        }
    }

    #[cfg(windows)]
    #[test]
    fn starts_shortcuts_and_packages_through_explorer_without_recording() {
        for (launch, expected) in [
            (
                AppLaunch {
                    target: r"C:\Apps\sample.exe".into(),
                    shortcut: Some(r"C:\Start Menu\Sample.lnk".into()),
                    ..Default::default()
                },
                r"C:\Start Menu\Sample.lnk".to_string(),
            ),
            (
                AppLaunch { aumid: Some("Package_Family!App".into()), ..Default::default() },
                r"shell:AppsFolder\Package_Family!App".to_string(),
            ),
        ] {
            let probe = FakeProbe::new("C:\\").with_path(r"C:\Start Menu\Sample.lnk");
            let LaunchPlan::Native(plan) = build(&probe, &meta(launch)) else { panic!("not a native plan") };
            assert_eq!(plan.command.program, "explorer.exe");
            assert_eq!(plan.command.args, [expected]);
            assert_eq!(plan.record_session, Some(false));
        }

        let direct = AppLaunch {
            target: r"C:\Apps\tool.exe".into(),
            arguments: "--flag \"with spaces\"".into(),
            working_dir: Some(r"C:\Apps".into()),
            ..Default::default()
        };
        let probe = FakeProbe::new("C:\\").with_path(r"C:\Apps\tool.exe").with_path(r"C:\Apps");
        let LaunchPlan::Native(plan) = build(&probe, &meta(direct)) else { panic!("not a native plan") };
        assert_eq!(plan.command.program, r"C:\Apps\tool.exe");
        assert_eq!(plan.command.args, ["--flag", "with spaces"]);
        assert_eq!(plan.command.cwd.as_deref(), Some(r"C:\Apps"));
        assert_eq!(plan.record_session, Some(false));
    }

    #[test]
    fn malformed_package_ids_are_rejected() {
        assert!(valid_aumid("Publisher.Name_abc!App"));
        assert!(!valid_aumid("Publisher!App"));
        assert!(!valid_aumid("Package_family!App & calc"));
    }
}
