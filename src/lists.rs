// SPDX-License-Identifier: AGPL-3.0-or-later

//! Item lists of the pages, as [`flaneur_vm::Paginator`]s running on the tokio runtime.
//!
//! A [`ListBinding`] is what a [`VideoGrid`](crate::widgets::VideoGrid) binds to: the paginator's
//! state receiver, and commands that run the paginator on the runtime. The builders below make
//! the lists of the pages:
//!
//! - home and search ask every connector that supports them at once and merge the answers with
//!   [`flaneur_vm::interleave`], equal weights, so each service keeps its own ranking;
//! - the subscription feed merges the items of the feeds followed locally (RSS) by date;
//! - the history comes from the local database;
//! - channel videos and related items come from one connector.

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::Arc;

use flaneur_connector::{
    ChannelTab, Connector, Error, FeedKind, SearchFilters, SearchResult, Support,
};
use flaneur_model::{ChannelRef, Cursor, Item, ItemKind, ItemRef, Page, WatchState};
use flaneur_store::HistoryEntry;
use flaneur_vm::{FeedSource, ListState, Paginator, Registry, interleave};
use futures_util::future::join_all;
use tokio::sync::watch;

use crate::runtime;
use crate::services::SharedStore;

/// Items of the local history per page.
const HISTORY_PAGE: u32 = 30;
/// Most items the subscription feed shows.
const SUBSCRIPTION_ITEMS: usize = 300;

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

    /// The items loaded so far.
    pub fn items(&self) -> Arc<Vec<Item>> {
        Arc::clone(&self.state.borrow().items)
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

type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;

/// One page of one connector's part of a merged list.
type Fetch = Arc<
    dyn Fn(Arc<dyn Connector>, Option<Cursor>) -> BoxFuture<flaneur_connector::Result<Page<Item>>>
        + Send
        + Sync,
>;

/// A list merged from several connectors. Its cursor is a JSON object of the cursors of the
/// connectors that have more, by service id; a connector that fails is left out of the page,
/// unless they all fail.
fn merged(connectors: Vec<Arc<dyn Connector>>, fetch: Fetch) -> ListBinding {
    let connectors: Arc<[Arc<dyn Connector>]> = connectors.into();
    ListBinding::from_paginator(Paginator::new(move |cursor: Option<Cursor>| {
        let connectors = Arc::clone(&connectors);
        let fetch = Arc::clone(&fetch);
        async move { merged_page(&connectors, &fetch, cursor).await }
    }))
}

async fn merged_page(
    connectors: &[Arc<dyn Connector>],
    fetch: &Fetch,
    cursor: Option<Cursor>,
) -> flaneur_connector::Result<Page<Item>> {
    let wanted: Vec<(Arc<dyn Connector>, Option<Cursor>)> = match cursor {
        None => connectors.iter().map(|c| (Arc::clone(c), None)).collect(),
        Some(cursor) => {
            let cursors: BTreeMap<String, String> =
                serde_json::from_str(&cursor.0).map_err(|e| Error::Parse(e.to_string()))?;
            connectors
                .iter()
                .filter_map(|c| {
                    let next = cursors.get(&c.info().id.0)?;
                    Some((Arc::clone(c), Some(Cursor(next.clone()))))
                })
                .collect()
        }
    };
    let answers = join_all(wanted.into_iter().map(|(connector, cursor)| {
        let page = fetch(Arc::clone(&connector), cursor);
        async move { (connector.info().id.clone(), page.await) }
    }))
    .await;

    let mut sources = Vec::new();
    let mut next = BTreeMap::new();
    let mut first_error = None;
    for (service, answer) in answers {
        match answer {
            Ok(page) => {
                if let Some(cursor) = page.next {
                    next.insert(service.0.clone(), cursor.0);
                }
                sources.push(FeedSource {
                    service,
                    items: page.items,
                    weight: 1,
                });
            }
            Err(error) => {
                tracing::warn!(%error, %service, "a service failed to answer");
                first_error.get_or_insert(error);
            }
        }
    }
    if sources.is_empty()
        && let Some(error) = first_error
    {
        return Err(error);
    }
    let next = (!next.is_empty())
        .then(|| serde_json::to_string(&next).map(Cursor))
        .transpose()
        .map_err(|e| Error::Parse(e.to_string()))?;
    Ok(Page {
        items: interleave(sources),
        next,
        total: None,
    })
}

/// Whether a connector can serve its home feed now.
fn offers_home(connector: &dyn Connector) -> bool {
    let caps = connector.capabilities();
    connector.feeds().is_some() && caps.home.available(caps.signed_in)
}

/// The homes of every connector that has one, interleaved; `None` when none has.
pub fn home(registry: &Registry) -> Option<ListBinding> {
    let connectors: Vec<_> = registry
        .iter()
        .filter(|c| offers_home(c.as_ref()))
        .cloned()
        .collect();
    if connectors.is_empty() {
        return None;
    }
    Some(merged(
        connectors,
        Arc::new(|connector, cursor| {
            Box::pin(async move {
                let feeds = connector.feeds().ok_or(Error::Unsupported)?;
                feeds.feed(FeedKind::Home, cursor.as_ref()).await
            })
        }),
    ))
}

/// The connectors that answer searches now.
fn search_connectors(registry: &Registry) -> Vec<Arc<dyn Connector>> {
    registry
        .iter()
        .filter(|c| {
            let caps = c.capabilities();
            c.search().is_some() && caps.search.available(caps.signed_in)
        })
        .cloned()
        .collect()
}

/// Whether some connector can search now.
pub fn can_search(registry: &Registry) -> bool {
    !search_connectors(registry).is_empty()
}

/// The playable results of a search on every connector that searches, interleaved. Channels and
/// playlists are left out of the video grid.
pub fn search(registry: &Registry, query: &str) -> Option<ListBinding> {
    let connectors = search_connectors(registry);
    if connectors.is_empty() {
        return None;
    }
    let query: Arc<str> = query.into();
    Some(merged(
        connectors,
        Arc::new(move |connector, cursor| {
            let query = Arc::clone(&query);
            Box::pin(async move {
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
            })
        }),
    ))
}

/// The connectors whose subscription feed Flâneur builds itself from locally followed channels.
fn local_feed_connectors(registry: &Registry) -> Vec<Arc<dyn Connector>> {
    registry
        .iter()
        .filter(|c| c.capabilities().subscription_feed == Support::Local && c.channels().is_some())
        .cloned()
        .collect()
}

/// Whether a connector keeps its follows in the local database.
pub fn follows_locally(connector: &dyn Connector) -> bool {
    connector.capabilities().subscriptions_write == Support::Local && connector.channels().is_some()
}

/// The latest items of every channel followed locally (RSS feeds), newest first; `None` when no
/// connector keeps local follows.
pub fn local_subscriptions(registry: &Registry, store: SharedStore) -> Option<ListBinding> {
    let connectors: Arc<[Arc<dyn Connector>]> = local_feed_connectors(registry).into();
    if connectors.is_empty() {
        return None;
    }
    Some(ListBinding::from_paginator(Paginator::new(
        move |_cursor: Option<Cursor>| {
            let connectors = Arc::clone(&connectors);
            let store = store.clone();
            async move {
                let mut followed = Vec::new();
                for connector in connectors.iter() {
                    let channels = store
                        .lock()
                        .local_subscriptions(&connector.info().id)
                        .map_err(|e| Error::Service(e.to_string()))?;
                    followed.extend(
                        channels
                            .into_iter()
                            .map(|c| (Arc::clone(connector), c.r#ref)),
                    );
                }
                let lists = join_all(followed.into_iter().map(|(connector, channel)| async move {
                    let channels = connector.channels().ok_or(Error::Unsupported)?;
                    let page = channels.items(&channel, ChannelTab::Videos, None).await;
                    if let Err(error) = &page {
                        tracing::warn!(%error, %channel, "a followed feed failed to load");
                    }
                    page
                }))
                .await;
                let items = lists.into_iter().flatten().flat_map(|page| page.items);
                Ok(Page::last(newest_first(items, SUBSCRIPTION_ITEMS)))
            }
        },
    )))
}

/// Merges items by publication date, newest first; undated items go last.
fn newest_first(items: impl IntoIterator<Item = Item>, limit: usize) -> Vec<Item> {
    let mut items: Vec<Item> = items.into_iter().collect();
    items.sort_by_key(|item| std::cmp::Reverse(item.published.map(|p| p.at)));
    items.truncate(limit);
    items
}

/// The local history, most recent first.
pub fn history(store: SharedStore) -> ListBinding {
    ListBinding::from_paginator(Paginator::new(move |cursor: Option<Cursor>| {
        let store = store.clone();
        async move {
            let before = cursor
                .map(|c| {
                    c.0.parse()
                        .map_err(|_| Error::Parse("history cursor".into()))
                })
                .transpose()?;
            let entries = store
                .lock()
                .history(HISTORY_PAGE, before)
                .map_err(|e| Error::Service(e.to_string()))?;
            let full = entries.len() == HISTORY_PAGE as usize;
            let next = entries
                .last()
                .filter(|_| full)
                .map(|e| Cursor(e.progress.updated_at.to_string()));
            Ok(Page {
                items: entries.into_iter().map(history_item).collect(),
                next,
                total: None,
            })
        }
    }))
}

/// The item of a history entry, with its progress.
fn history_item(entry: HistoryEntry) -> Item {
    let mut item = entry.item.unwrap_or_else(|| bare_item(entry.r#ref));
    item.watch = Some(WatchState {
        progress: Some(entry.progress),
        watched: Some(entry.watched),
        reaction: item.watch.and_then(|w| w.reaction),
    });
    item
}

/// An item known by its reference only.
pub fn bare_item(r#ref: ItemRef) -> Item {
    Item {
        title: r#ref.id.clone(),
        r#ref,
        kind: ItemKind::Video,
        description: None,
        duration: None,
        published: None,
        scheduled_start: None,
        channel: None,
        thumbnails: Vec::new(),
        dimensions: None,
        views: None,
        reactions: flaneur_model::ReactionCounts::default(),
        flags: flaneur_model::ContentFlags::default(),
        web_url: None,
        watch: None,
    }
}

/// The videos of a channel.
pub fn channel_videos(connector: Arc<dyn Connector>, channel: ChannelRef) -> ListBinding {
    ListBinding::from_paginator(Paginator::new(move |cursor: Option<Cursor>| {
        let connector = Arc::clone(&connector);
        let channel = channel.clone();
        async move {
            let channels = connector.channels().ok_or(Error::Unsupported)?;
            channels
                .items(&channel, ChannelTab::Videos, cursor.as_ref())
                .await
        }
    }))
}

/// The items related to an item, to play next.
pub fn related(connector: Arc<dyn Connector>, item: ItemRef) -> ListBinding {
    ListBinding::from_paginator(Paginator::new(move |cursor: Option<Cursor>| {
        let connector = Arc::clone(&connector);
        let item = item.clone();
        async move { connector.player().related(&item, cursor.as_ref()).await }
    }))
}

#[cfg(test)]
mod tests {
    use flaneur_model::{Precision, Progress, Published, ServiceId};
    use flaneur_store::Store;

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

    fn loaded(binding: &ListBinding) -> ListState<Item> {
        binding.load_more();
        wait_for(binding, |s| {
            !s.loading && (!s.items.is_empty() || !s.has_more || s.error.is_some())
        })
    }

    #[test]
    fn an_empty_registry_provides_no_list() {
        let registry = Registry::new();
        assert!(home(&registry).is_none());
        assert!(!can_search(&registry));
        assert!(search(&registry, "anything").is_none());
        let store = SharedStore::new(Store::in_memory().unwrap());
        assert!(local_subscriptions(&registry, store).is_none());
    }

    #[test]
    fn the_home_loads_page_after_page() {
        let home = home(&demo_registry()).expect("the demo has a home feed");
        let first = loaded(&home);
        home.load_more();
        let second = wait_for(&home, |s| s.items.len() > first.items.len() && !s.loading);
        assert!(second.has_more);
        assert_eq!(second.items[..first.items.len()], first.items[..]);
    }

    #[test]
    fn search_keeps_matching_items_only() {
        let results = search(&demo_registry(), "sintel").expect("the demo searches");
        let state = loaded(&results);
        assert!(!state.items.is_empty());
        assert!(
            state
                .items
                .iter()
                .all(|i| i.title.to_lowercase().contains("sintel"))
        );
    }

    #[test]
    fn subscriptions_merge_followed_channels_by_date() {
        let registry = demo_registry();
        let store = SharedStore::new(Store::in_memory().unwrap());
        let feed = local_subscriptions(&registry, store.clone()).expect("the demo follows locally");
        assert!(loaded(&feed).items.is_empty());

        let demo = registry.get(&ServiceId::new("demo")).unwrap();
        let channel = runtime::runtime()
            .block_on(
                demo.channels()
                    .unwrap()
                    .channel(&DemoConnector::channel_ref()),
            )
            .unwrap();
        store
            .lock()
            .add_local_subscription(&channel.summary, 0)
            .unwrap();
        feed.refresh();
        let state = wait_for(&feed, |s| !s.loading && !s.items.is_empty());
        let dates: Vec<_> = state
            .items
            .iter()
            .map(|i| i.published.map(|p| p.at))
            .collect();
        assert!(dates.windows(2).all(|w| w[0] >= w[1]));
    }

    #[test]
    fn newest_first_puts_undated_items_last() {
        let dated = |id: &str, at: Option<i64>| Item {
            published: at.map(|at| Published {
                at,
                precision: Precision::Exact,
            }),
            ..bare_item(ItemRef::new(ServiceId::new("t"), id))
        };
        let merged = newest_first(
            [dated("a", Some(1)), dated("b", None), dated("c", Some(3))],
            2,
        );
        let ids: Vec<_> = merged.iter().map(|i| i.r#ref.id.as_str()).collect();
        assert_eq!(ids, ["c", "a"]);
    }

    #[test]
    fn history_lists_played_items_with_their_progress() {
        let store = SharedStore::new(Store::in_memory().unwrap());
        for (id, at) in [("old", 10), ("new", 20)] {
            let progress = Progress {
                position: 30,
                duration: Some(60),
                updated_at: at,
            };
            store
                .lock()
                .record_progress(
                    &ItemRef::new(ServiceId::new("t"), id),
                    None,
                    progress,
                    false,
                )
                .unwrap();
        }
        let state = loaded(&history(store));
        let ids: Vec<_> = state.items.iter().map(|i| i.r#ref.id.as_str()).collect();
        assert_eq!(ids, ["new", "old"]);
        assert_eq!(
            state.items[0]
                .watch
                .and_then(|w| w.progress)
                .map(|p| p.position),
            Some(30)
        );
        assert!(!state.has_more);
    }
}
