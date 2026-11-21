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

use flaneur_connector::{Error, Result};
use flaneur_model::{ItemRef, Playable};
use flaneur_vm::Registry;

/// How many times an item may hand playback over to another service.
const MAX_HANDOFFS: usize = 3;

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

/// Resolves an item's streams with its connector, following handoffs: a page that only embeds a
/// video hosted on another service is resolved by that service's connector.
pub async fn resolve(registry: Registry, item: ItemRef) -> Result<Playable> {
    let mut item = item;
    for _ in 0..=MAX_HANDOFFS {
        let connector = registry
            .get(&item.service)
            .ok_or_else(|| Error::Unavailable(format!("no connector for {}", item.service)))?;
        let playable = connector.player().resolve(&item).await?;
        match playable.handoff.clone() {
            Some(next) => item = next,
            None => return Ok(playable),
        }
    }
    Err(Error::Unavailable(
        "too many handoffs between services".into(),
    ))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use flaneur_model::ServiceId;

    use super::*;
    use crate::demo::DemoConnector;

    #[test]
    fn resolves_with_the_item_service_and_reports_missing_ones() {
        let mut registry = Registry::new();
        registry.register(Arc::new(DemoConnector::new()));
        let runtime = crate::runtime::runtime();

        let playable = runtime
            .block_on(resolve(
                registry.clone(),
                ItemRef::new(ServiceId::new("demo"), "1"),
            ))
            .expect("the demo resolves its items");
        assert_eq!(playable.streams.len(), 1);

        let missing =
            runtime.block_on(resolve(registry, ItemRef::new(ServiceId::new("gone"), "1")));
        assert!(matches!(missing, Err(Error::Unavailable(_))));
    }
}
