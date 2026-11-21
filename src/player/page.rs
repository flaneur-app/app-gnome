// SPDX-License-Identifier: AGPL-3.0-or-later

//! The player page: the video, play/pause, seek, volume and fullscreen controls, and the related
//! items under them.
//!
//! Keyboard: Space or K plays and pauses, Left and Right seek 10 seconds, F toggles
//! fullscreen, Escape leaves it. In fullscreen the bars hide and come back on pointer motion.
//!
//! When the page plays an item of a service, it saves the position in the local database every
//! 10 seconds, when playback stops and at the end, and emits `ended` at the end so the window
//! can autoplay.

use std::cell::{Cell, RefCell};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use adw::prelude::*;
use adw::subclass::prelude::*;
use flaneur_model::{Item, Progress};
use gtk::glib::subclass::Signal;
use gtk::{CompositeTemplate, gdk, glib};

use super::{Engine, Event, Media};
use crate::lists::ListBinding;
use crate::widgets::VideoGrid;
use crate::{format, runtime, services};

/// How often the position is refreshed.
const TICK: Duration = Duration::from_millis(250);
/// How long position updates pause after the user moved the seek bar.
const SEEK_GRACE: Duration = Duration::from_millis(600);
/// How long the bars stay visible after the pointer stops, in fullscreen.
const CONTROLS_TIMEOUT: Duration = Duration::from_secs(3);
/// Step of the seek shortcuts, in seconds.
const SEEK_STEP: i64 = 10;
/// How often the position is saved while playing.
const SAVE_EVERY: Duration = Duration::from_secs(10);
/// Share of the duration after which an item counts as watched.
const WATCHED_AT: f64 = 0.9;

mod imp {
    use super::{
        Cell, CompositeTemplate, Engine, Instant, Item, OnceLock, RefCell, Signal, VideoGrid, gdk,
        glib,
    };
    use adw::prelude::*;
    use adw::subclass::prelude::*;

    #[derive(Default, CompositeTemplate)]
    #[template(resource = "/dev/baxyz/Flaneur/ui/player-page.ui")]
    pub struct PlayerPage {
        #[template_child]
        pub toolbar_view: TemplateChild<adw::ToolbarView>,
        #[template_child]
        pub picture: TemplateChild<gtk::Picture>,
        #[template_child]
        pub buffering: TemplateChild<adw::Spinner>,
        #[template_child]
        pub error_status: TemplateChild<adw::StatusPage>,
        #[template_child]
        pub play_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub position_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub seek_scale: TemplateChild<gtk::Scale>,
        #[template_child]
        pub duration_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub volume_button: TemplateChild<gtk::ScaleButton>,
        #[template_child]
        pub related_revealer: TemplateChild<gtk::Revealer>,
        #[template_child]
        pub related: TemplateChild<VideoGrid>,
        pub engine: RefCell<Option<Engine>>,
        /// The item playing, when it comes from a service.
        pub item: RefCell<Option<Item>>,
        /// Where to seek once the media is ready, in seconds.
        pub start_at: Cell<Option<u64>>,
        pub last_save: Cell<Option<Instant>>,
        pub bus_watch: RefCell<Option<gst::bus::BusWatchGuard>>,
        pub tick: RefCell<Option<glib::SourceId>>,
        pub wants_playing: Cell<bool>,
        pub live: Cell<bool>,
        /// When the user last sought, and to where, in seconds.
        pub last_user_seek: Cell<Option<(Instant, f64)>>,
        pub fullscreen_watch: RefCell<Option<(gtk::Window, glib::SignalHandlerId)>>,
        pub hide_controls: RefCell<Option<glib::SourceId>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for PlayerPage {
        const NAME: &'static str = "FlaneurPlayerPage";
        type Type = super::PlayerPage;
        type ParentType = adw::NavigationPage;

        fn class_init(klass: &mut Self::Class) {
            VideoGrid::ensure_type();
            klass.bind_template();
            klass.install_action("player.toggle-play", None, |page, _, _| page.toggle_play());
            klass.install_action("player.fullscreen", None, |page, _, _| {
                page.toggle_fullscreen();
            });
            klass.install_action("player.leave-fullscreen", None, |page, _, _| {
                page.leave_fullscreen();
            });
            klass.install_action("player.seek-forward", None, |page, _, _| {
                page.seek_by(super::SEEK_STEP);
            });
            klass.install_action("player.seek-backward", None, |page, _, _| {
                page.seek_by(-super::SEEK_STEP);
            });
            let none = gdk::ModifierType::empty();
            klass.add_binding_action(gdk::Key::space, none, "player.toggle-play");
            klass.add_binding_action(gdk::Key::k, none, "player.toggle-play");
            klass.add_binding_action(gdk::Key::f, none, "player.fullscreen");
            klass.add_binding_action(gdk::Key::Escape, none, "player.leave-fullscreen");
            klass.add_binding_action(gdk::Key::Right, none, "player.seek-forward");
            klass.add_binding_action(gdk::Key::Left, none, "player.seek-backward");
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for PlayerPage {
        fn signals() -> &'static [Signal] {
            static SIGNALS: OnceLock<Vec<Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| vec![Signal::builder("ended").build()])
        }

        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();
            self.related.set_strip();
            self.related.set_empty_state("Nothing Related", None, None);

            self.seek_scale.connect_change_value(glib::clone!(
                #[weak]
                obj,
                #[upgrade_or]
                glib::Propagation::Proceed,
                move |_, _, value| {
                    obj.seek_to(value);
                    glib::Propagation::Proceed
                }
            ));
            self.volume_button.connect_value_changed(glib::clone!(
                #[weak]
                obj,
                move |_, value| obj.set_volume(value)
            ));

            let double_click = gtk::GestureClick::new();
            double_click.connect_pressed(glib::clone!(
                #[weak]
                obj,
                move |_, presses, _, _| {
                    if presses == 2 {
                        obj.toggle_fullscreen();
                    }
                }
            ));
            self.picture.add_controller(double_click);

            let motion = gtk::EventControllerMotion::new();
            motion.connect_motion(glib::clone!(
                #[weak]
                obj,
                move |_, _, _| obj.show_controls_for_a_while()
            ));
            obj.add_controller(motion);
        }

        fn dispose(&self) {
            if let Some(source) = self.tick.take() {
                source.remove();
            }
            if let Some(source) = self.hide_controls.take() {
                source.remove();
            }
        }
    }

    impl WidgetImpl for PlayerPage {}

    impl NavigationPageImpl for PlayerPage {
        fn shown(&self) {
            self.parent_shown();
            self.obj().watch_fullscreen();
        }

        fn hidden(&self) {
            self.parent_hidden();
            self.obj().stop();
        }
    }
}

glib::wrapper! {
    /// The player page.
    pub struct PlayerPage(ObjectSubclass<imp::PlayerPage>)
        @extends adw::NavigationPage, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl PlayerPage {
    /// Plays a media, sending `headers` with its HTTP requests; the page shows `title` in its
    /// header. `item` is the item of a service being played (its position is saved), and
    /// `start_at` where to start, in seconds.
    pub fn load(
        &self,
        media: &Media,
        headers: &[(String, String)],
        title: &str,
        item: Option<Item>,
        start_at: Option<u64>,
    ) {
        let imp = self.imp();
        self.save_progress(false);
        imp.item.replace(item);
        imp.start_at.set(start_at.filter(|s| *s > 0));
        imp.last_save.set(Some(Instant::now()));
        self.set_title(title);
        imp.error_status.set_visible(false);
        imp.buffering.set_visible(true);
        imp.seek_scale.set_sensitive(false);
        imp.seek_scale.set_value(0.0);
        imp.position_label.set_label(&format::duration(0));
        imp.duration_label.set_label(&format::duration(0));

        if imp.engine.borrow().is_none() && !self.create_engine() {
            return;
        }
        let result = imp
            .engine
            .borrow()
            .as_ref()
            .map(|engine| engine.load(media, headers));
        match result {
            Some(Ok(live)) => {
                imp.live.set(live);
                imp.wants_playing.set(true);
                self.start_ticking();
            }
            Some(Err(error)) => self.show_error(&error.to_string()),
            None => {}
        }
    }

    fn create_engine(&self) -> bool {
        let imp = self.imp();
        let engine = match Engine::new() {
            Ok(engine) => engine,
            Err(error) => {
                self.show_error(&format!(
                    "The GStreamer elements the player needs are missing: {error}"
                ));
                return false;
            }
        };
        imp.picture.set_paintable(Some(engine.paintable()));
        engine.set_volume(imp.volume_button.value().powi(3));
        let watch = engine.watch(glib::clone!(
            #[weak(rename_to = page)]
            self,
            move |event| page.on_event(event)
        ));
        match watch {
            Ok(guard) => {
                imp.bus_watch.replace(Some(guard));
            }
            Err(error) => tracing::warn!(%error, "cannot watch the player bus"),
        }
        imp.engine.replace(Some(engine));
        true
    }

    fn on_event(&self, event: Event) {
        let imp = self.imp();
        match event {
            Event::State(state) => {
                let playing = state == gst::State::Playing;
                imp.play_button.set_icon_name(if playing {
                    "media-playback-pause-symbolic"
                } else {
                    "media-playback-start-symbolic"
                });
                imp.play_button
                    .set_tooltip_text(Some(if playing { "Pause" } else { "Play" }));
                if state >= gst::State::Paused {
                    imp.buffering.set_visible(false);
                    if let Some(start) = imp.start_at.take()
                        && let Some(engine) = imp.engine.borrow().as_ref()
                    {
                        tracing::info!(start, "resuming");
                        engine.seek(gst::ClockTime::from_seconds(start));
                    }
                    self.update_position();
                }
            }
            Event::Buffering(percent) => {
                tracing::debug!(percent, "buffering");
                imp.buffering.set_visible(percent < 100);
                // Live streams cannot pause; on-demand ones pause until the buffer is full.
                if !imp.live.get()
                    && let Some(engine) = imp.engine.borrow().as_ref()
                {
                    engine.set_playing(percent >= 100 && imp.wants_playing.get());
                }
            }
            Event::DurationChanged => self.update_position(),
            Event::Eos => {
                imp.wants_playing.set(false);
                if let Some(engine) = imp.engine.borrow().as_ref() {
                    engine.set_playing(false);
                }
                self.save_progress(true);
                self.emit_by_name::<()>("ended", &[]);
            }
            Event::Error(message) => self.show_error(&message),
        }
    }

    fn show_error(&self, message: &str) {
        let imp = self.imp();
        imp.buffering.set_visible(false);
        imp.error_status.set_description(Some(message));
        imp.error_status.set_visible(true);
    }

    fn start_ticking(&self) {
        let imp = self.imp();
        if imp.tick.borrow().is_some() {
            return;
        }
        let source = glib::timeout_add_local(
            TICK,
            glib::clone!(
                #[weak(rename_to = page)]
                self,
                #[upgrade_or]
                glib::ControlFlow::Break,
                move || {
                    page.update_position();
                    glib::ControlFlow::Continue
                }
            ),
        );
        imp.tick.replace(Some(source));
    }

    fn update_position(&self) {
        let imp = self.imp();
        let Some((position, duration)) = imp
            .engine
            .borrow()
            .as_ref()
            .map(|e| (e.position(), e.duration()))
        else {
            return;
        };
        if self.pending_seek().is_some() {
            return;
        }
        let position = position.map_or(0, gst::ClockTime::seconds);
        tracing::trace!(position, "player position");
        if imp.wants_playing.get()
            && imp
                .last_save
                .get()
                .is_none_or(|at| at.elapsed() >= SAVE_EVERY)
        {
            self.save_progress(false);
        }
        imp.position_label.set_label(&format::duration(position));
        if let Some(duration) = duration.filter(|d| !d.is_zero()) {
            let seconds = duration.seconds();
            imp.duration_label.set_label(&format::duration(seconds));
            let adjustment = imp.seek_scale.adjustment();
            adjustment.set_upper(duration.seconds_f64());
            imp.seek_scale.set_sensitive(true);
            #[allow(clippy::cast_precision_loss, reason = "seconds of a video fit in f64")]
            imp.seek_scale.set_value(position as f64);
        }
    }

    fn seek_to(&self, seconds: f64) {
        let imp = self.imp();
        imp.last_user_seek.set(Some((Instant::now(), seconds)));
        if let Some(engine) = imp.engine.borrow().as_ref() {
            engine.seek(gst::ClockTime::from_seconds_f64(seconds.max(0.0)));
        }
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "clamped above zero"
        )]
        imp.position_label
            .set_label(&format::duration(seconds.max(0.0) as u64));
    }

    /// The target of a seek the pipeline may not have reached yet.
    fn pending_seek(&self) -> Option<f64> {
        self.imp()
            .last_user_seek
            .get()
            .filter(|(at, _)| at.elapsed() < SEEK_GRACE)
            .map(|(_, target)| target)
    }

    fn seek_by(&self, delta: i64) {
        let imp = self.imp();
        // Repeated shortcuts add up from the pending target: right after a flushing seek, the
        // pipeline still reports the old position.
        let position = self.pending_seek().or_else(|| {
            imp.engine
                .borrow()
                .as_ref()
                .and_then(Engine::position)
                .map(gst::ClockTime::seconds_f64)
        });
        let Some(position) = position else { return };
        #[allow(clippy::cast_precision_loss, reason = "a few seconds fit in f64")]
        self.seek_to(position + delta as f64);
    }

    fn toggle_play(&self) {
        let imp = self.imp();
        let Some(times) = imp
            .engine
            .borrow()
            .as_ref()
            .map(|e| (e.position(), e.duration()))
        else {
            return;
        };
        let playing = !imp.wants_playing.get();
        // Playing again after the end starts over.
        if playing
            && let (Some(position), Some(duration)) = times
            && position + gst::ClockTime::from_mseconds(500) >= duration
        {
            self.seek_to(0.0);
        }
        imp.wants_playing.set(playing);
        if let Some(engine) = imp.engine.borrow().as_ref() {
            engine.set_playing(playing);
        }
    }

    fn set_volume(&self, value: f64) {
        // The slider is perceptual; GStreamer's volume is linear (cubic mapping, like
        // `GST_STREAM_VOLUME_FORMAT_CUBIC`).
        if let Some(engine) = self.imp().engine.borrow().as_ref() {
            engine.set_volume(value.powi(3));
        }
    }

    fn window(&self) -> Option<gtk::Window> {
        self.root().and_downcast::<gtk::Window>()
    }

    fn toggle_fullscreen(&self) {
        if let Some(window) = self.window() {
            window.set_fullscreened(!window.is_fullscreen());
        }
    }

    fn leave_fullscreen(&self) {
        if let Some(window) = self.window() {
            window.unfullscreen();
        }
    }

    /// Hides the bars while the window is fullscreen.
    fn watch_fullscreen(&self) {
        let imp = self.imp();
        if imp.fullscreen_watch.borrow().is_some() {
            return;
        }
        let Some(window) = self.window() else { return };
        let handler = window.connect_fullscreened_notify(glib::clone!(
            #[weak(rename_to = page)]
            self,
            move |window| page.set_bars_revealed(!window.is_fullscreen())
        ));
        self.set_bars_revealed(!window.is_fullscreen());
        imp.fullscreen_watch.replace(Some((window, handler)));
    }

    fn set_bars_revealed(&self, revealed: bool) {
        let imp = self.imp();
        imp.toolbar_view.set_reveal_top_bars(revealed);
        imp.toolbar_view.set_reveal_bottom_bars(revealed);
        // The related items stay hidden in fullscreen, where only the controls come back.
        let fullscreen = self.window().is_some_and(|w| w.is_fullscreen());
        imp.related_revealer
            .set_reveal_child(!fullscreen && imp.related.binding().is_some());
    }

    fn show_controls_for_a_while(&self) {
        let imp = self.imp();
        if !self.window().is_some_and(|w| w.is_fullscreen()) {
            return;
        }
        self.set_bars_revealed(true);
        if let Some(source) = imp.hide_controls.take() {
            source.remove();
        }
        let source = glib::timeout_add_local_once(
            CONTROLS_TIMEOUT,
            glib::clone!(
                #[weak(rename_to = page)]
                self,
                move || {
                    page.imp().hide_controls.take();
                    if page.window().is_some_and(|w| w.is_fullscreen()) {
                        page.set_bars_revealed(false);
                    }
                }
            ),
        );
        imp.hide_controls.replace(Some(source));
    }

    /// Shows the related items of the current one; `None` hides the strip.
    pub fn set_related(&self, related: Option<&ListBinding>) {
        let imp = self.imp();
        if let Some(binding) = related {
            imp.related.bind(binding);
        }
        imp.related_revealer.set_reveal_child(related.is_some());
    }

    /// The related items shown, if any.
    pub fn related(&self) -> Option<ListBinding> {
        self.imp().related.binding()
    }

    /// The related items' grid, to react to activated items.
    pub fn related_grid(&self) -> VideoGrid {
        self.imp().related.get()
    }

    /// The item playing, when it comes from a service.
    pub fn item(&self) -> Option<Item> {
        self.imp().item.borrow().clone()
    }

    /// Connects to `ended`, emitted at the end of the media.
    pub fn connect_ended<F: Fn(&Self) + 'static>(&self, f: F) -> glib::SignalHandlerId {
        self.connect_closure(
            "ended",
            false,
            glib::closure_local!(move |page: &Self| f(page)),
        )
    }

    /// Saves the position of the item playing in the local database, in the background.
    fn save_progress(&self, ended: bool) {
        let imp = self.imp();
        let Some(item) = imp.item.borrow().clone() else {
            return;
        };
        let Some(store) = services::get().store.clone() else {
            return;
        };
        let Some((position, duration)) = imp
            .engine
            .borrow()
            .as_ref()
            .map(|e| (e.position(), e.duration()))
        else {
            return;
        };
        let duration = duration
            .map(|d| u32::try_from(d.seconds()).unwrap_or(u32::MAX))
            .filter(|d| *d > 0);
        let position = match (ended, duration) {
            (true, Some(duration)) => duration,
            _ => position.map_or(0, |p| u32::try_from(p.seconds()).unwrap_or(u32::MAX)),
        };
        if position == 0 && !ended {
            return;
        }
        imp.last_save.set(Some(Instant::now()));
        let watched =
            ended || duration.is_some_and(|d| f64::from(position) >= f64::from(d) * WATCHED_AT);
        let progress = Progress {
            position,
            duration,
            updated_at: services::now(),
        };
        runtime::spawn(async move {
            if let Err(error) =
                store
                    .lock()
                    .record_progress(&item.r#ref, Some(&item), progress, watched)
            {
                tracing::warn!(%error, "saving the position");
            }
        });
    }

    /// Stops playback and leaves fullscreen; called when the page is closed.
    fn stop(&self) {
        let imp = self.imp();
        self.save_progress(false);
        imp.wants_playing.set(false);
        if let Some(engine) = imp.engine.borrow().as_ref() {
            engine.stop();
        }
        if let Some(source) = imp.tick.take() {
            source.remove();
        }
        if let Some(source) = imp.hide_controls.take() {
            source.remove();
        }
        if let Some((window, handler)) = imp.fullscreen_watch.take() {
            window.disconnect(handler);
            window.unfullscreen();
        }
        self.set_bars_revealed(true);
    }
}
