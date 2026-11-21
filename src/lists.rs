// SPDX-License-Identifier: AGPL-3.0-or-later

//! Item lists of the pages, as [`flaneur_vm::Paginator`]s running on the tokio runtime.
//!
//! A [`ListBinding`] is what a [`VideoGrid`](crate::widgets::VideoGrid) binds to: the paginator's
//! state receiver, and commands that run the paginator on the runtime. The source builders pick
//! the connector that provides a list. Until the aggregating view-models of the core exist
//! (`HomeVm`, `SubscriptionsVm`, `SearchVm`), each list comes from the first connector, by id,
//! that supports it right now.

use std::future::Future;
use std::rc::Rc;
use std::sync::Arc;

use flaneur_connector::{Connector, Error, FeedKind, SearchFilters, SearchResult};
use flaneur_model::{Cursor, Item, Page};
use flaneur_vm::{ListState, Paginator, Registry};
use tokio::sync::watch;

use crate::runtime;

/// A paginated list of items, observed from the GTK main loop.
#[derive(Clone)]
pub struct ListBinding {
    state: watch::Receiver<ListState<Item>>,
    load_more: Rc<dyn Fn()>,
    refresh: Rc<dyn Fn()>,
}

impl ListBinding {
    /// Wraps a paginator; its loads run on the tokio runtime.
    pub fn from_paginator<F, Fut>(paginator: Paginator<Item, F>) -> Self
    where
        F: Fn(Option<Cursor>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Page<Item>, Error>> + Send + 'static,
    {
        let paginator = Arc::new(paginator);
        let state = paginator.subscribe();
        let load_more = {
            let paginator = Arc::clone(&paginator);
            Rc::new(move || {
                let paginator = Arc::clone(&paginator);
                runtime::spawn(async move { paginator.load_more().await });
            })
        };
        let refresh = Rc::new(move || {
            let paginator = Arc::clone(&paginator);
            runtime::spawn(async move { paginator.refresh().await });
        });
        Self {
            state,
            load_more,
            refresh,
        }
    }

    /// A receiver of the list state.
    pub fn state(&self) -> watch::Receiver<ListState<Item>> {
        self.state.clone()
    }

    /// Loads the next page in the background; does nothing while a page is loading.
    pub fn load_more(&self) {
        (self.load_more)();
    }

    /// Reloads the list from its first page, in the background.
    pub fn refresh(&self) {
        (self.refresh)();
    }
}

/// Whether a connector can serve a feed now.
fn offers_feed(connector: &dyn Connector, kind: FeedKind) -> bool {
    let caps = connector.capabilities();
    let support = match kind {
        FeedKind::Home => caps.home,
        FeedKind::Subscriptions => caps.subscription_feed,
        FeedKind::Trending => caps.trending,
        FeedKind::Shorts => caps.shorts,
    };
    connector.feeds().is_some() && support.available(caps.signed_in)
}

/// A feed of the first connector that offers it; `None` when no connector does.
pub fn feed(registry: &Registry, kind: FeedKind) -> Option<ListBinding> {
    let connector = Arc::clone(registry.iter().find(|c| offers_feed(c.as_ref(), kind))?);
    Some(ListBinding::from_paginator(Paginator::new(
        move |cursor: Option<Cursor>| {
            let connector = Arc::clone(&connector);
            async move {
                let feeds = connector.feeds().ok_or(Error::Unsupported)?;
                feeds.feed(kind, cursor.as_ref()).await
            }
        },
    )))
}

/// The watch history of the first connector that can read it.
pub fn history(registry: &Registry) -> Option<ListBinding> {
    let connector = Arc::clone(registry.iter().find(|c| {
        let caps = c.capabilities();
        c.library().is_some() && caps.history_read.available(caps.signed_in)
    })?);
    Some(ListBinding::from_paginator(Paginator::new(
        move |cursor: Option<Cursor>| {
            let connector = Arc::clone(&connector);
            async move {
                let library = connector.library().ok_or(Error::Unsupported)?;
                library.history(cursor.as_ref()).await
            }
        },
    )))
}

/// The connector that answers searches, if any.
fn search_connector(registry: &Registry) -> Option<&Arc<dyn Connector>> {
    registry.iter().find(|c| {
        let caps = c.capabilities();
        c.search().is_some() && caps.search.available(caps.signed_in)
    })
}

/// Whether some connector can search now.
pub fn can_search(registry: &Registry) -> bool {
    search_connector(registry).is_some()
}

/// The playable results of a search; channels and playlists are left out of the video grid.
pub fn search(registry: &Registry, query: &str) -> Option<ListBinding> {
    let connector = Arc::clone(search_connector(registry)?);
    let query: Arc<str> = query.into();
    Some(ListBinding::from_paginator(Paginator::new(
        move |cursor: Option<Cursor>| {
            let connector = Arc::clone(&connector);
            let query = Arc::clone(&query);
            async move {
                let search = connector.search().ok_or(Error::Unsupported)?;
                let page = search
                    .search(&query, &SearchFilters::default(), cursor.as_ref())
                    .await?;
                Ok(Page {
                    items: page
                        .items
                        .into_iter()
                        .filter_map(|result| match result {
                            SearchResult::Item(item) => Some(*item),
                            SearchResult::Channel(_) | SearchResult::Playlist(_) => None,
                        })
                        .collect(),
                    next: page.next,
                    total: None,
                })
            }
        },
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demo::DemoConnector;

    fn demo_registry() -> Registry {
        let mut registry = Registry::new();
        registry.register(Arc::new(DemoConnector::new()));
        registry
    }

    fn wait_for(
        binding: &ListBinding,
        done: impl FnMut(&ListState<Item>) -> bool,
    ) -> ListState<Item> {
        let mut state = binding.state();
        runtime::runtime().block_on(async move {
            state
                .wait_for(done)
                .await
                .expect("the paginator is alive")
                .clone()
        })
    }

    #[test]
    fn an_empty_registry_provides_no_list() {
        let registry = Registry::new();
        assert!(feed(&registry, FeedKind::Home).is_none());
        assert!(history(&registry).is_none());
        assert!(!can_search(&registry));
        assert!(search(&registry, "anything").is_none());
    }

    #[test]
    fn a_registered_feed_loads_page_after_page() {
        let registry = demo_registry();
        assert!(feed(&registry, FeedKind::Trending).is_none());
        let home = feed(&registry, FeedKind::Home).expect("the demo has a home feed");

        home.load_more();
        let first = wait_for(&home, |s| !s.items.is_empty() && !s.loading);
        home.load_more();
        let second = wait_for(&home, |s| s.items.len() > first.items.len() && !s.loading);
        assert!(second.has_more);
        assert_eq!(second.items[..first.items.len()], first.items[..]);
    }

    #[test]
    fn search_keeps_matching_items_only() {
        let registry = demo_registry();
        let results = search(&registry, "sintel").expect("the demo searches");
        results.load_more();
        let state = wait_for(&results, |s| {
            !s.loading && (!s.items.is_empty() || !s.has_more)
        });
        assert!(!state.items.is_empty());
        assert!(
            state
                .items
                .iter()
                .all(|i| i.title.to_lowercase().contains("sintel"))
        );
    }
}
