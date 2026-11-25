use std::collections::BTreeMap;

use heronite_platform::registry::{self, Hive, Value};

use crate::InstalledProgram;

const UNINSTALL: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall";
const UNINSTALL_WOW: &str = r"Software\Wow6432Node\Microsoft\Windows\CurrentVersion\Uninstall";

pub(super) fn installed_programs() -> Vec<InstalledProgram> {
    let mut entries = Vec::new();
    for (hive, key) in [(Hive::LocalMachine, UNINSTALL), (Hive::LocalMachine, UNINSTALL_WOW), (Hive::CurrentUser, UNINSTALL)] {
        for name in registry::subkeys(hive, key) {
            let full_key = format!("{key}\\{name}");
            let Some(values) = registry::values(hive, &full_key) else { continue };
            if let Some(entry) = parse_entry(values) {
                entries.push(entry);
            }
        }
    }
    entries
}

fn parse_entry(values: Vec<(String, Value)>) -> Option<InstalledProgram> {
    let values: BTreeMap<String, Value> = values.into_iter().map(|(k, v)| (k.to_ascii_lowercase(), v)).collect();
    let text = |key: &str| match values.get(key) {
        Some(Value::Str(s)) if !s.trim().is_empty() => Some(s.trim().to_string()),
        _ => None,
    };
    let dword = |key: &str| match values.get(key) {
        Some(Value::Dword(v)) => Some(*v),
        Some(Value::Str(v)) => v.trim().parse::<u32>().ok(),
        _ => None,
    };
    if dword("systemcomponent") == Some(1) || text("parentkeyname").is_some() || text("releasetype").is_some() {
        return None;
    }
    let display_name = text("displayname")?;
    Some(InstalledProgram {
        display_name,
        publisher: text("publisher"),
        install_location: text("installlocation"),
        display_icon: text("displayicon"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn excludes_system_components_parent_entries_and_updates() {
        let entry = |fields: &[(&str, Value)]| parse_entry(fields.iter().map(|(key, value)| ((*key).into(), value.clone())).collect());
        let base = [("DisplayName", Value::Str("App".into()))];
        assert!(entry(&base).is_some());
        assert!(entry(&[base[0].clone(), ("SystemComponent", Value::Dword(1))]).is_none());
        assert!(entry(&[base[0].clone(), ("ParentKeyName", Value::Str("parent".into()))]).is_none());
        assert!(entry(&[base[0].clone(), ("ReleaseType", Value::Str("Update".into()))]).is_none());
        assert!(entry(&[("Publisher", Value::Str("Publisher".into()))]).is_none());
    }
}
