use hmeta_model::InstalledApplication;
use std::collections::BTreeMap;

pub(crate) fn normalize_installed_applications(
    applications: Vec<InstalledApplication>,
) -> Vec<InstalledApplication> {
    let mut by_bundle = BTreeMap::<String, InstalledApplication>::new();
    for application in applications {
        let bundle_name = application.bundle_name.trim();
        if bundle_name.is_empty() {
            continue;
        }
        let name = application.name.trim();
        by_bundle.entry(bundle_name.to_owned()).or_insert_with(|| {
            let name = if name.is_empty() { bundle_name } else { name };
            InstalledApplication {
                bundle_name: bundle_name.to_owned(),
                name: name.to_owned(),
            }
        });
    }
    let mut applications = by_bundle.into_values().collect::<Vec<_>>();
    applications.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then_with(|| a.bundle_name.cmp(&b.bundle_name))
    });
    applications
}

pub(crate) fn matches_installed_application_query(
    application: &InstalledApplication,
    query: &str,
) -> bool {
    let query = query.trim().to_ascii_lowercase();
    if query.is_empty() {
        return true;
    }
    contains(&application.name, &query) || contains(&application.bundle_name, &query)
}

fn contains(value: &str, needle: &str) -> bool {
    value.to_ascii_lowercase().contains(needle)
}
