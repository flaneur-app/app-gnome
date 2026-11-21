// SPDX-License-Identifier: AGPL-3.0-or-later

//! Thumbnails and avatars, loaded in the background and kept in a small in-memory cache.
//!
//! Downloads run on the tokio runtime, a few at a time, and so does decoding: JPEG, PNG and WebP
//! (what Odysee's CDN serves, which GDK cannot always load) are decoded by the `image` crate and
//! scaled down to what a card shows. The GTK thread only wraps the pixels in a texture and hands
//! it to the widgets that asked for it. Several widgets asking for the same image share one
//! download. The cache keeps the most recently used textures in memory; nothing goes to disk.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::sync::OnceLock;
use std::time::Duration;

use flaneur_model::Thumbnail;
use gtk::prelude::*;
use gtk::{gdk, glib};
use tokio::sync::Semaphore;

use crate::runtime;

/// Textures kept in memory (about 0.5 MB each at the largest size).
const CACHE_SIZE: usize = 160;
/// Largest size images are scaled down to, in pixels: a card's thumbnail on a 2x screen.
const MAX_WIDTH: u32 = 480;
const MAX_HEIGHT: u32 = 270;
/// Downloads at the same time.
const PARALLEL_DOWNLOADS: usize = 6;

type Callback<T> = Box<dyn FnOnce(&T)>;

/// Images by URL, the least recently used one forgotten first.
struct Cache<T> {
    textures: HashMap<String, T>,
    /// Least recently used first.
    order: VecDeque<String>,
    /// Callbacks waiting for a download, by URL.
    pending: HashMap<String, Vec<Callback<T>>>,
}

impl<T> Default for Cache<T> {
    fn default() -> Self {
        Self {
            textures: HashMap::new(),
            order: VecDeque::new(),
            pending: HashMap::new(),
        }
    }
}

impl<T: Clone> Cache<T> {
    fn get(&mut self, url: &str) -> Option<T> {
        let texture = self.textures.get(url)?.clone();
        if let Some(index) = self.order.iter().position(|u| u == url) {
            self.order.remove(index);
        }
        self.order.push_back(url.to_owned());
        Some(texture)
    }

    fn insert(&mut self, url: String, texture: T) {
        if self.textures.insert(url.clone(), texture).is_none() {
            self.order.push_back(url);
        }
        while self.order.len() > CACHE_SIZE {
            if let Some(old) = self.order.pop_front() {
                self.textures.remove(&old);
            }
        }
    }
}

thread_local! {
    static CACHE: RefCell<Cache<gdk::Texture>> = RefCell::default();
}

fn client() -> &'static (reqwest::Client, Semaphore) {
    static CLIENT: OnceLock<(reqwest::Client, Semaphore)> = OnceLock::new();
    CLIENT.get_or_init(|| {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .user_agent(concat!(
                "Flaneur/",
                env!("CARGO_PKG_VERSION"),
                " (+https://flaneur.baxyz.dev)"
            ))
            .build()
            .unwrap_or_default();
        (client, Semaphore::new(PARALLEL_DOWNLOADS))
    })
}

/// The URL of the thumbnail that best fits `width` logical pixels on a HiDPI screen.
pub fn pick(thumbnails: &[Thumbnail], width: u32) -> Option<String> {
    Thumbnail::best_for_width(thumbnails, width * 2).map(|t| t.url.to_string())
}

/// Calls `done` with the image at `url`, from the cache or once downloaded. Must run on the GTK
/// thread. Failures are logged and `done` is dropped.
pub fn load(url: &str, done: impl FnOnce(&gdk::Texture) + 'static) {
    let cached = CACHE.with_borrow_mut(|cache| cache.get(url));
    if let Some(texture) = cached {
        done(&texture);
        return;
    }
    let first = CACHE.with_borrow_mut(|cache| {
        let waiting = cache.pending.entry(url.to_owned()).or_default();
        waiting.push(Box::new(done));
        waiting.len() == 1
    });
    if !first {
        return;
    }

    let owned = url.to_owned();
    let download = runtime::spawn(async move {
        let (client, slots) = client();
        let _slot = slots.acquire().await.map_err(|e| e.to_string())?;
        let response = client
            .get(&owned)
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(|e| e.to_string())?;
        let bytes = response.bytes().await.map_err(|e| e.to_string())?;
        tokio::task::spawn_blocking(move || decode(&bytes))
            .await
            .map_err(|e| e.to_string())?
    });
    let url = url.to_owned();
    glib::spawn_future_local(async move {
        let image = download.await.map_err(|e| e.to_string()).and_then(|r| r);
        let texture = image.map(|image| {
            let (width, height) = image.dimensions();
            let stride = width as usize * 4;
            gdk::MemoryTexture::new(
                i32::try_from(width).unwrap_or(i32::MAX),
                i32::try_from(height).unwrap_or(i32::MAX),
                gdk::MemoryFormat::R8g8b8a8,
                &glib::Bytes::from_owned(image.into_raw()),
                stride,
            )
            .upcast::<gdk::Texture>()
        });
        let waiting = CACHE.with_borrow_mut(|cache| {
            if let Ok(texture) = &texture {
                cache.insert(url.clone(), texture.clone());
            }
            cache.pending.remove(&url).unwrap_or_default()
        });
        match texture {
            Ok(texture) => waiting.into_iter().for_each(|done| done(&texture)),
            Err(error) => tracing::debug!(%error, %url, "image not loaded"),
        }
    });
}

/// Decodes an image and scales it down to fit a card.
fn decode(bytes: &[u8]) -> Result<image::RgbaImage, String> {
    let image = image::load_from_memory(bytes).map_err(|e| e.to_string())?;
    let image = if image.width() > MAX_WIDTH || image.height() > MAX_HEIGHT {
        image.thumbnail(MAX_WIDTH, MAX_HEIGHT)
    } else {
        image
    };
    Ok(image.into_rgba8())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cache_forgets_the_least_recently_used_texture() {
        let mut cache = Cache::default();
        for i in 0..=CACHE_SIZE {
            cache.insert(i.to_string(), i);
            if i == 1 {
                // Using "0" again makes "1" the oldest.
                assert!(cache.get("0").is_some());
            }
        }
        assert_eq!(cache.textures.len(), CACHE_SIZE);
        assert!(cache.get("0").is_some());
        assert!(cache.get("1").is_none());
    }
    #[test]
    fn decoding_scales_large_images_down() {
        let mut png = Vec::new();
        image::RgbaImage::new(1280, 720)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let image = decode(&png).unwrap();
        assert_eq!(image.dimensions(), (480, 270));
        assert!(decode(b"not an image").is_err());
    }
}
