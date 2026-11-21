// SPDX-License-Identifier: AGPL-3.0-or-later

//! The connectors this build ships, and the local database.
//!
//! Connectors are chosen at build time; there is no plugin loading. Adding one is a dependency
//! in `Cargo.toml` and one line in [`registry`]. Every page reads what to show from the registry:
//! a connector with a home feed fills the home, one with search answers searches, and so on.
//!
//! The app's state that belongs to no service (resume positions, the history, the RSS feeds the
//! user follows) lives in a [`Store`]: one SQLite file in the user data directory, which is the
//! app's own data directory inside the Flatpak sandbox. Secrets never go there.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};

use flaneur_connector::{Connector, Error, Result};
use flaneur_model::{ItemRef, Playable, Seconds, ServiceId};
use flaneur_store::Store;
use flaneur_vm::Registry;
use gtk::glib;

/// How many times an item may hand playback over to another service.
const MAX_HANDOFFS: usize = 3;

/// What the app talks to: the connectors and the local database.
pub struct Services {
    /// The connectors of this build.
    pub registry: Registry,
    /// The local database; `None` when it cannot be opened (the app then keeps nothing).
    pub store: Option<SharedStore>,
    names: BTreeMap<ServiceId, String>,
}

/// The store, shared with the tokio runtime. Calls are short SQLite statements; run them on the
/// runtime, not on the GTK thread.
#[derive(Clone)]
pub struct SharedStore(Arc<Mutex<Store>>);

impl SharedStore {
    /// Wraps a store.
    pub fn new(store: Store) -> Self {
        Self(Arc::new(Mutex::new(store)))
    }

    /// Locks the store; a panic while it was locked leaves it usable (SQLite stays consistent).
    pub fn lock(&self) -> MutexGuard<'_, Store> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The services of the running app, set up on first use.
pub fn get() -> &'static Services {
    static SERVICES: OnceLock<Services> = OnceLock::new();
    SERVICES.get_or_init(|| {
        let registry = registry();
        let names = registry
            .iter()
            .map(|c| (c.info().id.clone(), c.info().name.clone()))
            .collect();
        Services {
            registry,
            store: open_store(),
            names,
        }
    })
}

impl Services {
    /// The display name of a service ("YouTube", "RSS & Atom").
    pub fn name(&self, service: &ServiceId) -> String {
        self.names
            .get(service)
            .cloned()
            .unwrap_or_else(|| service.to_string())
    }

    /// The connector of a service.
    pub fn connector(&self, service: &ServiceId) -> Option<Arc<dyn Connector>> {
        self.registry.get(service).cloned()
    }
}

/// Builds the registry of the connectors of this build. A connector that fails to start is left
/// out, with a warning.
pub fn registry() -> Registry {
    // Connectors build HTTP clients, which may need the runtime's reactor.
    let _runtime = crate::runtime::runtime().enter();
    let mut registry = Registry::new();

    let youtube = flaneur_connector_youtube::YouTube::new(flaneur_connector_youtube::Config {
        cache_file: cache_dir().map(|dir| dir.join("rustypipe_cache.json")),
        botguard: false,
    });
    add(&mut registry, "youtube", youtube);
    add(
        &mut registry,
        "odysee",
        flaneur_connector_odysee::Odysee::new(),
    );
    add(
        &mut registry,
        "rss",
        flaneur_connector_rss::RssConnector::new(),
    );
    #[cfg(feature = "demo")]
    registry.register(Arc::new(crate::demo::DemoConnector::new()));
    registry
}

fn add<C: Connector + 'static>(registry: &mut Registry, name: &str, connector: Result<C>) {
    match connector {
        Ok(connector) => registry.register(Arc::new(connector)),
        Err(error) => tracing::warn!(%error, connector = name, "connector left out"),
    }
}

/// `$XDG_CACHE_HOME/flaneur`, created if needed.
fn cache_dir() -> Option<PathBuf> {
    let dir = glib::user_cache_dir().join("flaneur");
    std::fs::create_dir_all(&dir)
        .inspect_err(|error| tracing::warn!(%error, "no cache directory"))
        .ok()?;
    Some(dir)
}

/// Opens `$XDG_DATA_HOME/flaneur/flaneur.sqlite`.
fn open_store() -> Option<SharedStore> {
    let dir = glib::user_data_dir().join("flaneur");
    let path = dir.join("flaneur.sqlite");
    let opened = std::fs::create_dir_all(&dir)
        .map_err(|e| e.to_string())
        .and_then(|()| Store::open(&path).map_err(|e| e.to_string()));
    match opened {
        Ok(store) => {
            tracing::info!(path = %path.display(), "local database");
            Some(SharedStore::new(store))
        }
        Err(error) => {
            tracing::warn!(%error, path = %path.display(), "local database unavailable");
            None
        }
    }
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

/// Where to resume an item: the local position, unless it was watched or barely started or
/// ended. Call it on the runtime.
pub fn resume_position(store: &SharedStore, item: &ItemRef) -> Option<Seconds> {
    let entry = store
        .lock()
        .progress(item)
        .inspect_err(|error| tracing::warn!(%error, "reading the resume position"))
        .ok()??;
    let position = entry.progress.position;
    let near_end = entry.progress.duration.is_some_and(|d| position + 10 >= d);
    (!entry.watched && position >= 5 && !near_end).then_some(position)
}

/// The current time, in Unix seconds.
pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_secs()).ok())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use flaneur_model::Progress;

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

    #[test]
    fn resumes_only_from_the_middle_of_unwatched_items() {
        let store = SharedStore::new(Store::in_memory().unwrap());
        let item = ItemRef::new(ServiceId::new("demo"), "1");
        let record = |position, watched| {
            let progress = Progress {
                position,
                duration: Some(600),
                updated_at: now() + i64::from(position),
            };
            store
                .lock()
                .record_progress(&item, None, progress, watched)
                .unwrap();
        };
        assert_eq!(resume_position(&store, &item), None);
        record(3, false);
        assert_eq!(resume_position(&store, &item), None);
        record(120, false);
        assert_eq!(resume_position(&store, &item), Some(120));
        record(595, false);
        assert_eq!(resume_position(&store, &item), None);
        record(596, true);
        assert_eq!(resume_position(&store, &item), None);
    }
}
