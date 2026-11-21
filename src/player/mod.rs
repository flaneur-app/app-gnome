// SPDX-License-Identifier: AGPL-3.0-or-later

//! Video playback with GStreamer.
//!
//! [`Engine`] wraps a `playbin3` whose video sink is `gtk4paintablesink`; [`PlayerPage`] shows
//! its paintable in a `GtkPicture` inside a `GtkGraphicsOffload`, so on Wayland the compositor
//! can scan decoded frames out directly when the decoder produces dmabufs.
//!
//! # Hardware decoding
//!
//! The app selects no decoder itself: `playbin3` autoplugs the highest-ranked decoder for each
//! stream. The VA-API decoders of the `va` plugin (gst-plugins-bad: `vah264dec`, `vavp9dec`,
//! `vaav1dec`, ...) are registered only when a VA device and driver exist, with the primary rank
//! (GStreamer 1.22 and later), above the software decoders, so they win wherever the GPU can
//! decode the codec.
//! The older `gstreamer-vaapi` plugin (`vaapih264dec`, ...) is deprecated and not expected.
//! In the Flatpak, the GNOME runtime ships the `va` plugin; `--device=dri` gives access to the
//! GPU and the `org.freedesktop.Platform.GL` extensions bring the Mesa VA drivers (Intel's
//! `iHD` driver comes from the `org.freedesktop.Platform.VAAPI.Intel` extension). To force or
//! forbid a decoder while testing, use `GST_PLUGIN_FEATURE_RANK`, for instance
//! `GST_PLUGIN_FEATURE_RANK=vah264dec:NONE` to compare with software decoding.
//!
//! [`hardware_codecs`] reports which codecs have a hardware decoder, so the stream selector of
//! the core ranks them first (a codec the GPU cannot decode falls back to the CPU and stutters).

mod engine;
mod page;
mod proxy;

use flaneur_model::VideoCodec;
use flaneur_vm::Selection;
use gst::prelude::*;

pub use engine::{Engine, Event};
pub use page::PlayerPage;

/// What to play.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Media {
    /// One URI: a file, a stream with audio and video, or an adaptive manifest (HLS, DASH).
    Uri(String),
    /// Separate video and audio streams of one item, played in sync.
    Split {
        /// Video stream.
        video: String,
        /// Audio stream.
        audio: String,
    },
}

impl Media {
    /// The same media, with the URLs of CDNs that only serve bounded ranges going through the
    /// local chunking proxy (see [`proxy`]), which sends `headers` upstream.
    #[must_use]
    pub fn chunked(self, headers: &[(String, String)]) -> Self {
        let route = |url: String| {
            if proxy::needs_chunks(&url) {
                proxy::register(&url, headers)
            } else {
                url
            }
        };
        match self {
            Self::Uri(uri) => Self::Uri(route(uri)),
            Self::Split { video, audio } => Self::Split {
                video: route(video),
                audio: route(audio),
            },
        }
    }
}

impl From<Selection<'_>> for Media {
    fn from(selection: Selection<'_>) -> Self {
        match selection {
            Selection::Split { video, audio } => Self::Split {
                video: video.url.to_string(),
                audio: audio.url.to_string(),
            },
            Selection::Single(stream) => Self::Uri(stream.url.to_string()),
            Selection::Hls(manifest) => Self::Uri(manifest.to_string()),
        }
    }
}

/// Codecs with a hardware decoder on this machine.
///
/// A decoder element of the `va`, `nvcodec` or `v4l2codecs` plugins is only registered when its
/// device exists, so its presence means the GPU can decode the codec.
pub fn hardware_codecs() -> Vec<VideoCodec> {
    let decoders = [
        (
            VideoCodec::H264,
            ["vah264dec", "nvh264dec", "v4l2slh264dec"],
        ),
        (
            VideoCodec::H265,
            ["vah265dec", "nvh265dec", "v4l2slh265dec"],
        ),
        (VideoCodec::Vp9, ["vavp9dec", "nvvp9dec", "v4l2slvp9dec"]),
        (VideoCodec::Av1, ["vaav1dec", "nvav1dec", "v4l2slav1dec"]),
    ];
    decoders
        .into_iter()
        .filter(|(_, elements)| {
            elements.iter().any(|name| {
                gst::ElementFactory::find(name).is_some_and(|f| f.rank() > gst::Rank::NONE)
            })
        })
        .map(|(codec, _)| codec)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use flaneur_model::{Container, Stream, StreamKind};
    use url::Url;

    fn stream(url: &str, kind: StreamKind) -> Stream {
        Stream {
            url: Url::parse(url).unwrap(),
            kind,
            container: Container::WebM,
            bitrate: None,
            size: None,
            width: None,
            height: None,
            fps: None,
            video_codec: None,
            hdr: None,
            audio: None,
            label: None,
        }
    }

    #[test]
    fn split_selection_keeps_both_streams() {
        let video = stream("https://cdn.example/v", StreamKind::Video);
        let audio = stream("https://cdn.example/a", StreamKind::Audio);
        assert_eq!(
            Media::from(Selection::Split {
                video: &video,
                audio: &audio
            }),
            Media::Split {
                video: "https://cdn.example/v".into(),
                audio: "https://cdn.example/a".into(),
            }
        );
        let hls = Url::parse("https://cdn.example/live.m3u8").unwrap();
        assert_eq!(
            Media::from(Selection::Hls(&hls)),
            Media::Uri("https://cdn.example/live.m3u8".into())
        );
    }
}
