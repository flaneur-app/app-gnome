// SPDX-License-Identifier: AGPL-3.0-or-later

//! The connectors this build ships.
//!
//! Connectors are chosen at build time; there is no plugin loading. Adding one is a dependency
//! in `Cargo.toml` and one line in [`registry`]:
//!
//! ```ignore
//! registry.register(Arc::new(flaneur_connector_rss::RssConnector::new()));
//! ```
//!
//! Every page of the app reads what to show from the registry: a connector with a feed fills
//! the home, one with search enables the search page, and so on. With no connector, every page
//! shows its "No services connected yet" state.

use flaneur_vm::Registry;

/// Builds the registry of the connectors of this build.
pub fn registry() -> Registry {
    #[allow(
        unused_mut,
        reason = "only the demo feature registers a connector so far"
    )]
    let mut registry = Registry::new();
    #[cfg(feature = "demo")]
    registry.register(std::sync::Arc::new(crate::demo::DemoConnector::new()));
    registry
}
