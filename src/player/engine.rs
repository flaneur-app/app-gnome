// SPDX-License-Identifier: AGPL-3.0-or-later

//! The GStreamer pipeline of the player.
//!
//! # Separate video and audio streams
//!
//! Services often serve video and audio as separate streams ([`Media::Split`], from
//! `flaneur_vm::Selection::Split`). `playbin3` plays them as one item: `uri` takes the video and
//! `suburi` the audio. Despite its name, `suburi` is not limited to subtitles in `playbin3`:
//! `uridecodebin3` opens one `urisourcebin` per URI and feeds both into a single `decodebin3`,
//! which merges their stream collections, selects one video and one audio stream, and plays them
//! against the pipeline clock; a seek reaches both sources. This was checked with a video-only
//! MP4 and an audio-only M4A cut from the same file: both streams are selected and stay in sync.
//!
//! A locally generated DASH manifest listing both streams (played by `dashdemux2`) is the
//! alternative to move to if the player ever needs to switch qualities mid-stream; for one fixed
//! pair of streams, `suburi` keeps the pipeline simple.

use gst::prelude::*;
use gtk::gdk;
use gtk::glib;

use super::Media;

/// What the pipeline reports to the page.
#[derive(Debug, Clone)]
pub enum Event {
    /// The pipeline reached a new state.
    State(gst::State),
    /// Network buffering progress, in percent.
    Buffering(i32),
    /// The duration is known, or changed.
    DurationChanged,
    /// The end of the media.
    Eos,
    /// Playback failed.
    Error(String),
}

/// A `playbin3` rendering into a GDK paintable.
pub struct Engine {
    playbin: gst::Element,
    paintable: gdk::Paintable,
}

impl Engine {
    /// Builds the pipeline. Needs a GDK display: the sink creates its GL context from it.
    pub fn new() -> Result<Self, glib::BoolError> {
        let playbin = gst::ElementFactory::make("playbin3")
            .name("flaneur-player")
            .build()?;
        let gtk_sink = gst::ElementFactory::make("gtk4paintablesink").build()?;
        let paintable = gtk_sink.property::<gdk::Paintable>("paintable");

        // With a GL context, `glsinkbin` uploads and converts frames on the GPU; without one
        // (software rendering, Broadway), the sink takes system memory frames.
        let has_gl = paintable
            .property::<Option<gdk::GLContext>>("gl-context")
            .is_some();
        let video_sink = if has_gl {
            gst::ElementFactory::make("glsinkbin")
                .property("sink", &gtk_sink)
                .build()
                .unwrap_or(gtk_sink)
        } else {
            gtk_sink
        };
        tracing::debug!(has_gl, "video sink ready");
        playbin.set_property("video-sink", &video_sink);
        Ok(Self { playbin, paintable })
    }

    /// The paintable showing the video.
    pub fn paintable(&self) -> &gdk::Paintable {
        &self.paintable
    }

    /// Reports pipeline events to `on_event` on the main loop, as long as the guard lives.
    pub fn watch<F: Fn(Event) + 'static>(
        &self,
        on_event: F,
    ) -> Result<gst::bus::BusWatchGuard, glib::BoolError> {
        let bus = self.playbin.bus().expect("a pipeline has a bus");
        let playbin = self.playbin.downgrade();
        bus.add_watch_local(move |_, message| {
            use gst::MessageView;
            let Some(playbin) = playbin.upgrade() else {
                return glib::ControlFlow::Break;
            };
            match message.view() {
                MessageView::StateChanged(change)
                    if message.src() == Some(playbin.upcast_ref()) =>
                {
                    tracing::info!(from = ?change.old(), to = ?change.current(), "player state");
                    on_event(Event::State(change.current()));
                }
                MessageView::Buffering(buffering) => {
                    on_event(Event::Buffering(buffering.percent()));
                }
                MessageView::DurationChanged(_) => on_event(Event::DurationChanged),
                MessageView::Eos(_) => {
                    tracing::info!("end of stream");
                    on_event(Event::Eos);
                }
                MessageView::StreamsSelected(selected) => {
                    for stream in selected.streams() {
                        tracing::info!(
                            kind = ?stream.stream_type(),
                            caps = %stream.caps().map_or_else(String::new, |c| c.to_string()),
                            "stream selected"
                        );
                    }
                }
                MessageView::Warning(warning) => {
                    tracing::warn!(warning = %warning.error(), debug = ?warning.debug(), "playback warning");
                }
                MessageView::Error(error) => {
                    tracing::warn!(error = %error.error(), debug = ?error.debug(), "playback error");
                    on_event(Event::Error(error.error().to_string()));
                }
                _ => {}
            }
            glib::ControlFlow::Continue
        })
    }

    /// Loads a media and starts playing it. Returns whether it is live (live sources do not
    /// preroll and must not be paused for buffering).
    pub fn load(&self, media: &Media) -> Result<bool, gst::StateChangeError> {
        self.playbin.set_state(gst::State::Null)?;
        let (uri, audio) = match media {
            Media::Uri(uri) => (uri.as_str(), None),
            Media::Split { video, audio } => (video.as_str(), Some(audio.as_str())),
        };
        tracing::info!(uri, audio, "loading media");
        self.playbin.set_property("uri", uri);
        self.playbin.set_property("suburi", audio);
        let success = self.playbin.set_state(gst::State::Playing)?;
        Ok(success == gst::StateChangeSuccess::NoPreroll)
    }

    /// Plays or pauses.
    pub fn set_playing(&self, playing: bool) {
        let state = if playing {
            gst::State::Playing
        } else {
            gst::State::Paused
        };
        if let Err(error) = self.playbin.set_state(state) {
            tracing::warn!(%error, ?state, "state change failed");
        }
    }

    /// Stops and frees the decoders and the network connections.
    pub fn stop(&self) {
        if let Err(error) = self.playbin.set_state(gst::State::Null) {
            tracing::warn!(%error, "stopping failed");
        }
    }

    /// Current position.
    pub fn position(&self) -> Option<gst::ClockTime> {
        self.playbin.query_position::<gst::ClockTime>()
    }

    /// Duration, once known; `None` for lives.
    pub fn duration(&self) -> Option<gst::ClockTime> {
        self.playbin.query_duration::<gst::ClockTime>()
    }

    /// Seeks to a position, on the nearest keyframe for responsiveness.
    pub fn seek(&self, position: gst::ClockTime) {
        if let Err(error) = self
            .playbin
            .seek_simple(gst::SeekFlags::FLUSH | gst::SeekFlags::KEY_UNIT, position)
        {
            tracing::warn!(%error, %position, "seek failed");
        }
    }

    /// Sets the volume, linear, from 0 to 1.
    pub fn set_volume(&self, volume: f64) {
        self.playbin.set_property("volume", volume.clamp(0.0, 1.0));
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.stop();
    }
}
