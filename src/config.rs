// SPDX-License-Identifier: AGPL-3.0-or-later

//! Build-time identity of the app.

/// Application id. Development builds (the Flatpak `Devel` manifest) set `FLANEUR_APP_ID` to
/// `dev.baxyz.Flaneur.Devel` so they install next to a stable build.
pub const APP_ID: &str = match option_env!("FLANEUR_APP_ID") {
    Some(id) => id,
    None => "dev.baxyz.Flaneur",
};

/// Where the bundled resources live, whatever the app id.
pub const RESOURCE_BASE_PATH: &str = "/dev/baxyz/Flaneur";

/// Version of this build.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Project website.
pub const WEBSITE: &str = "https://flaneur.baxyz.dev";

/// Bug tracker.
pub const ISSUE_URL: &str = "https://github.com/flaneur-app/app-gnome/issues";

/// A development build: the window gets the striped `devel` header bar.
pub fn is_devel() -> bool {
    APP_ID.ends_with(".Devel")
}
