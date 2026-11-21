// SPDX-License-Identifier: AGPL-3.0-or-later

//! A built-in fake service, to exercise the pages before real connectors exist.
//!
//! Compiled for the tests and with the `demo` cargo feature (`cargo run --features demo`). Its
//! items all play the Sintel trailer from the Blender Foundation (CC BY 3.0).

use async_trait::async_trait;
use flaneur_connector::{
    Capabilities, Connector, Credentials, Error, FeedKind, Feeds, Player, Result, Search,
    SearchFilters, SearchResult, ServiceInfo, Session, Support, Target,
};
use flaneur_model::{
    ChannelRef, ChannelSummary, Container, ContentFlags, Cursor, Item, ItemKind, ItemRef, Page,
    Playable, Precision, Published, ReactionCounts, ServiceId, Stream, StreamKind, VideoCodec,
};
use url::Url;

/// A public-domain-friendly test video, reachable over HTTPS.
pub const SAMPLE_URI: &str = "https://download.blender.org/durian/trailer/sintel_trailer-480p.mp4";

const PAGE_SIZE: usize = 12;
const PAGES: usize = 3;

/// The demo connector.
pub struct DemoConnector {
    info: ServiceInfo,
}

impl DemoConnector {
    /// The demo service.
    pub fn new() -> Self {
        Self {
            info: ServiceInfo {
                id: ServiceId::new("demo"),
                name: "Demo".into(),
                auth_methods: Vec::new(),
            },
        }
    }

    fn item(&self, index: usize) -> Item {
        let kind = if index % 7 == 3 {
            ItemKind::Live
        } else {
            ItemKind::Video
        };
        let title = if index.is_multiple_of(3) {
            format!("Sintel trailer, take {}", index + 1)
        } else {
            format!("Demo video {}", index + 1)
        };
        Item {
            r#ref: ItemRef::new(self.info.id.clone(), index.to_string()),
            kind,
            title,
            description: None,
            duration: (kind == ItemKind::Video).then_some(52),
            published: Some(Published {
                at: 1_288_000_000,
                precision: Precision::Day,
            }),
            scheduled_start: None,
            channel: Some(ChannelSummary {
                r#ref: ChannelRef::new(self.info.id.clone(), "blender"),
                name: "Blender Foundation".into(),
                avatars: Vec::new(),
                verified: None,
            }),
            thumbnails: Vec::new(),
            views: u64::try_from(index).ok().map(|i| i * 1_234),
            reactions: ReactionCounts::default(),
            flags: ContentFlags::default(),
            web_url: None,
            watch: None,
        }
    }

    fn page(&self, cursor: Option<&Cursor>) -> Result<Page<Item>> {
        let page: usize = match cursor {
            None => 0,
            Some(c) => c.0.parse().map_err(|_| Error::Parse("bad cursor".into()))?,
        };
        let start = page * PAGE_SIZE;
        Ok(Page {
            items: (start..start + PAGE_SIZE).map(|i| self.item(i)).collect(),
            next: (page + 1 < PAGES).then(|| Cursor((page + 1).to_string())),
            total: None,
        })
    }
}

#[async_trait]
impl Connector for DemoConnector {
    fn info(&self) -> &ServiceInfo {
        &self.info
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            home: Support::Anonymous,
            search: Support::Anonymous,
            ..Capabilities::default()
        }
    }

    async fn sign_in(&self, _credentials: Credentials) -> Result<Session> {
        Err(Error::Unsupported)
    }

    async fn restore(&self, _session: &Session) -> Result<()> {
        Err(Error::Unsupported)
    }

    fn sign_out(&self) {}

    fn parse_url(&self, _url: &Url) -> Option<Target> {
        None
    }

    fn player(&self) -> &dyn Player {
        self
    }

    fn feeds(&self) -> Option<&dyn Feeds> {
        Some(self)
    }

    fn search(&self) -> Option<&dyn Search> {
        Some(self)
    }
}

#[async_trait]
impl Player for DemoConnector {
    async fn resolve(&self, item: &ItemRef) -> Result<Playable> {
        let index = item.id.parse().map_err(|_| Error::NotFound)?;
        Ok(Playable {
            item: self.item(index),
            streams: vec![Stream {
                url: Url::parse(SAMPLE_URI).map_err(|e| Error::Parse(e.to_string()))?,
                kind: StreamKind::Muxed,
                container: Container::Mp4,
                bitrate: None,
                size: None,
                width: Some(854),
                height: Some(480),
                fps: Some(24),
                video_codec: Some(VideoCodec::H264),
                hdr: Some(false),
                audio: None,
            }],
            hls_manifest: None,
            dash_manifest: None,
            expires_at: None,
            start_at: None,
            tracking: None,
        })
    }

    async fn related(&self, _item: &ItemRef, _cursor: Option<&Cursor>) -> Result<Page<Item>> {
        Ok(Page::last(Vec::new()))
    }
}

#[async_trait]
impl Feeds for DemoConnector {
    async fn feed(&self, kind: FeedKind, cursor: Option<&Cursor>) -> Result<Page<Item>> {
        match kind {
            FeedKind::Home => self.page(cursor),
            FeedKind::Subscriptions | FeedKind::Trending | FeedKind::Shorts => {
                Err(Error::Unsupported)
            }
        }
    }
}

#[async_trait]
impl Search for DemoConnector {
    async fn search(
        &self,
        query: &str,
        _filters: &SearchFilters,
        cursor: Option<&Cursor>,
    ) -> Result<Page<SearchResult>> {
        let query = query.to_lowercase();
        let page = self.page(cursor)?;
        Ok(Page {
            items: page
                .items
                .into_iter()
                .filter(|item| item.title.to_lowercase().contains(&query))
                .map(|item| SearchResult::Item(Box::new(item)))
                .collect(),
            next: page.next,
            total: None,
        })
    }
}
