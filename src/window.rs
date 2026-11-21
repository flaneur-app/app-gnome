// SPDX-License-Identifier: AGPL-3.0-or-later

//! The main window: a sidebar of sections beside their content; the player and channel pages on
//! top.
//!
//! The navigation split view collapses into a single pane under 600sp, so the window fits a
//! phone-sized screen. The player and channel pages are pushed on the outer navigation view and
//! take the whole window.
//!
//! Playing an item: its connector resolves it (following handoffs to another service), the
//! core's selector picks the streams (hardware-decodable codecs first, 1080p at most), and the
//! player starts where the local history left off. At the end, the play queue picks the first
//! related item not played yet.

use std::cell::{OnceCell, RefCell};

use adw::prelude::*;
use adw::subclass::prelude::*;
use flaneur_connector::Target;
use flaneur_model::{ChannelRef, ChannelSummary, Item, Playable, ServiceId};
use flaneur_vm::{PlayQueue, SelectionPrefs, select_streams};
use gtk::{CompositeTemplate, gio, glib};

use crate::application::FlaneurApplication;
use crate::pages::{AccountsPage, ChannelPage, FeedPage, SearchPage};
use crate::player::{self, Media, PlayerPage};
use crate::widgets::VideoGrid;
use crate::{lists, runtime, services};

/// Shown where a list needs an account of YouTube or Odysee.
const SIGN_IN_LATER: &str =
    "Signing in to YouTube and Odysee comes in a later version: their own lists are not here yet";

mod imp {
    use super::{
        AccountsPage, ChannelRef, ChannelSummary, CompositeTemplate, FeedPage, OnceCell, PlayQueue,
        PlayerPage, RefCell, SearchPage, SelectionPrefs, ServiceId, glib,
    };
    use adw::prelude::*;
    use adw::subclass::prelude::*;

    #[derive(Default, CompositeTemplate)]
    #[template(resource = "/dev/baxyz/Flaneur/ui/window.ui")]
    pub struct FlaneurWindow {
        #[template_child]
        pub toast_overlay: TemplateChild<adw::ToastOverlay>,
        #[template_child]
        pub navigation: TemplateChild<adw::NavigationView>,
        #[template_child]
        pub split_view: TemplateChild<adw::NavigationSplitView>,
        #[template_child]
        pub sidebar: TemplateChild<gtk::ListBox>,
        #[template_child]
        pub content_page: TemplateChild<adw::NavigationPage>,
        #[template_child]
        pub add_feed_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub home_page: TemplateChild<FeedPage>,
        #[template_child]
        pub subscriptions_page: TemplateChild<FeedPage>,
        #[template_child]
        pub history_page: TemplateChild<FeedPage>,
        #[template_child]
        pub search_page: TemplateChild<SearchPage>,
        #[template_child]
        pub accounts_page: TemplateChild<AccountsPage>,
        #[template_child]
        pub player_page: TemplateChild<PlayerPage>,
        pub selection_prefs: OnceCell<SelectionPrefs>,
        pub queue: RefCell<PlayQueue>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for FlaneurWindow {
        const NAME: &'static str = "FlaneurWindow";
        type Type = super::FlaneurWindow;
        type ParentType = adw::ApplicationWindow;

        fn class_init(klass: &mut Self::Class) {
            FeedPage::ensure_type();
            SearchPage::ensure_type();
            AccountsPage::ensure_type();
            PlayerPage::ensure_type();
            klass.bind_template();
            klass.install_action("win.show-accounts", None, |window, _, _| {
                window.show_section("accounts");
            });
            klass.install_action("win.add-feed", None, |window, _, _| window.add_feed());
            klass.install_action("win.refresh-subscriptions", None, |window, _, _| {
                window.load_subscriptions();
            });
            klass.install_action(
                "win.show-channel",
                Some(&<(String, String, String)>::static_variant_type()),
                |window, _, target| {
                    if let Some((service, id, name)) =
                        target.and_then(<(String, String, String)>::from_variant)
                    {
                        window.show_channel(&ChannelSummary {
                            r#ref: ChannelRef::new(ServiceId::new(service), id),
                            name,
                            avatars: Vec::new(),
                            verified: None,
                        });
                    }
                },
            );
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for FlaneurWindow {
        fn constructed(&self) {
            self.parent_constructed();
            self.queue.replace(PlayQueue::new());
        }
    }

    impl WidgetImpl for FlaneurWindow {}
    impl WindowImpl for FlaneurWindow {}
    impl ApplicationWindowImpl for FlaneurWindow {}
    impl AdwApplicationWindowImpl for FlaneurWindow {}
}

glib::wrapper! {
    /// The main window.
    pub struct FlaneurWindow(ObjectSubclass<imp::FlaneurWindow>)
        @extends adw::ApplicationWindow, gtk::ApplicationWindow, gtk::Window, gtk::Widget,
        @implements gio::ActionGroup, gio::ActionMap, gtk::Accessible, gtk::Buildable,
            gtk::ConstraintTarget, gtk::Native, gtk::Root, gtk::ShortcutManager;
}

impl FlaneurWindow {
    /// A window showing the lists of the app's connectors.
    pub fn new(app: &FlaneurApplication) -> Self {
        let window: Self = glib::Object::builder().property("application", app).build();
        window.setup();
        window
    }

    fn setup(&self) {
        let imp = self.imp();
        if crate::config::is_devel() {
            self.add_css_class("devel");
        }
        let services = services::get();
        let registry = &services.registry;

        imp.home_page.set_empty_description(
            "The home brings together the recommendations of every service you connect.",
        );
        imp.home_page.set_source(lists::home(registry));

        imp.subscriptions_page.set_empty_description(
            "Follow the RSS or Atom feeds of sites that publish videos to see their latest ones.",
        );
        imp.subscriptions_page.set_notice(Some(SIGN_IN_LATER));
        imp.subscriptions_page.grid().set_empty_state(
            "No Feeds Followed Yet",
            Some(
                "Add the feed of a site that publishes videos, or the address of one of its pages.",
            ),
            Some(("_Add Feed", "win.add-feed")),
        );
        self.load_subscriptions();

        imp.history_page.set_empty_description(
            "What you watch, whatever the service, with the position to resume from.",
        );
        imp.history_page.set_notice(Some(SIGN_IN_LATER));
        imp.history_page.grid().set_empty_state(
            "Nothing Watched Yet",
            Some("Videos you play appear here, and resume where you left them."),
            None,
        );
        imp.history_page
            .set_source(services.store.clone().map(lists::history));

        imp.search_page.set_registry(registry.clone());
        imp.accounts_page.set_registry(registry);

        let grids: [VideoGrid; 5] = [
            imp.home_page.grid(),
            imp.subscriptions_page.grid(),
            imp.history_page.grid(),
            imp.search_page.grid(),
            imp.player_page.related_grid(),
        ];
        for grid in grids {
            self.play_from(&grid);
        }
        imp.player_page.connect_ended(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_| window.autoplay()
        ));

        imp.sidebar.set_header_func(|row, _| {
            row.set_header(
                (row.widget_name() == "accounts")
                    .then(|| gtk::Separator::new(gtk::Orientation::Horizontal))
                    .as_ref(),
            );
        });
        imp.sidebar.connect_row_activated(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_, row| {
                window.show_section(&row.widget_name());
                window.imp().split_view.set_show_content(true);
            }
        ));

        self.show_section("home");
    }

    /// Plays the items activated in a grid.
    fn play_from(&self, grid: &VideoGrid) {
        grid.connect_item_activated(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_, object| window.play_item(object.item())
        ));
    }

    /// Shows a section of the sidebar: `home`, `subscriptions`, `history`, `search` or
    /// `accounts`.
    pub fn show_section(&self, name: &str) {
        let imp = self.imp();
        let Some(child) = imp.stack.child_by_name(name) else {
            return;
        };
        imp.stack.set_visible_child(&child);
        if let Some(title) = imp.stack.page(&child).title() {
            imp.content_page.set_title(&title);
        }
        let mut index = 0;
        while let Some(row) = imp.sidebar.row_at_index(index) {
            if row.widget_name() == name {
                imp.sidebar.select_row(Some(&row));
                break;
            }
            index += 1;
        }
        imp.add_feed_button
            .set_visible(name == "subscriptions" && services::get().store.is_some());
        imp.split_view.set_show_content(true);
        match name {
            "search" => imp.search_page.focus_entry(),
            // The history changes with every video played.
            "history" => imp.history_page.grid().refresh(),
            _ => {}
        }
    }

    /// (Re)loads the subscription feed from the followed feeds.
    fn load_subscriptions(&self) {
        let services = services::get();
        let source = services
            .store
            .clone()
            .and_then(|store| lists::local_subscriptions(&services.registry, store));
        self.imp().subscriptions_page.set_source(source);
    }

    /// Shows a short message at the bottom of the window.
    pub fn toast(&self, message: &str) {
        self.imp().toast_overlay.add_toast(adw::Toast::new(message));
    }

    /// Opens the page of a channel.
    pub fn show_channel(&self, channel: &ChannelSummary) {
        let page = ChannelPage::new(channel);
        self.play_from(&page.grid());
        self.imp().navigation.push(&page);
    }

    /// Resolves an item's streams with its connector (following handoffs), then plays it from
    /// where the local history left off.
    fn play_item(&self, item: &Item) {
        let services = services::get();
        let registry = services.registry.clone();
        let store = services.store.clone();
        let item_ref = item.r#ref.clone();
        let task = runtime::spawn(async move {
            let resume = store
                .as_ref()
                .and_then(|store| services::resume_position(store, &item_ref));
            services::resolve(registry, item_ref)
                .await
                .map(|playable| (playable, resume))
        });
        let item = item.clone();
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            async move {
                match task.await {
                    Ok(Ok((playable, resume))) => window.play_playable(item, &playable, resume),
                    Ok(Err(error)) => {
                        tracing::warn!(%error, "cannot play");
                        window.toast(&format!("Cannot play this video: {error}"));
                    }
                    Err(error) => tracing::error!(%error, "stream resolution panicked"),
                }
            }
        ));
    }

    fn play_playable(&self, item: Item, playable: &Playable, resume: Option<u32>) {
        let imp = self.imp();
        let prefs = imp.selection_prefs.get_or_init(|| {
            let hardware_codecs = player::hardware_codecs();
            tracing::info!(?hardware_codecs, "hardware decoders");
            SelectionPrefs {
                hardware_codecs,
                ..SelectionPrefs::default()
            }
        });
        // The cap compares heights, so it rules out portrait videos (720×1280): play the best
        // stream there is rather than nothing.
        let uncapped = SelectionPrefs {
            max_height: u32::MAX,
            ..prefs.clone()
        };
        let Some(selection) =
            select_streams(playable, prefs).or_else(|| select_streams(playable, &uncapped))
        else {
            let streams: Vec<_> = playable
                .streams
                .iter()
                .map(|s| (s.kind, s.height, s.video_codec.clone()))
                .collect();
            tracing::warn!(item = %item.r#ref, ?streams, "no stream to play");
            self.toast("This video has no stream Flâneur can play");
            return;
        };
        let media = Media::from(selection).chunked(&playable.headers);
        tracing::info!(item = %item.r#ref, ?media, "playing");
        let start = resume.or(playable.start_at).map(u64::from);
        // A resolved item carries the full details; keep the listing's title otherwise.
        let title = if playable.item.title.is_empty() {
            item.title.clone()
        } else {
            playable.item.title.clone()
        };
        imp.queue.borrow_mut().play_now(item.clone());

        let related = services::get()
            .connector(&playable.item.r#ref.service)
            .filter(|c| {
                let caps = c.capabilities();
                caps.related.available(caps.signed_in)
            })
            .map(|c| lists::related(c, playable.item.r#ref.clone()));
        imp.player_page.set_related(related.as_ref());
        self.open_player(&media, &playable.headers, &title, Some(item), start);
    }

    /// Plays the first related item not played yet, when the media ends.
    fn autoplay(&self) {
        let imp = self.imp();
        let Some(related) = imp.player_page.related() else {
            return;
        };
        let items = related.items();
        let next = imp.queue.borrow().autoplay_candidate(&items).cloned();
        if let Some(next) = next {
            tracing::info!(item = %next.r#ref, "autoplay");
            self.play_item(&next);
        }
    }

    /// Opens the player on a media given on the command line.
    pub fn play_uri(&self, media: &Media, title: &str) {
        self.imp().player_page.set_related(None);
        self.open_player(media, &[], title, None, None);
    }

    fn open_player(
        &self,
        media: &Media,
        headers: &[(String, String)],
        title: &str,
        item: Option<Item>,
        start: Option<u64>,
    ) {
        let imp = self.imp();
        imp.player_page.load(media, headers, title, item, start);
        let player: &adw::NavigationPage = imp.player_page.upcast_ref();
        if imp.navigation.visible_page().as_ref() != Some(player) {
            imp.navigation.push(player);
        }
    }

    /// Asks for a feed (or a page that advertises one) and follows it.
    fn add_feed(&self) {
        let entry = gtk::Entry::builder()
            .placeholder_text("https://example.org/feed.xml")
            .activates_default(true)
            .input_purpose(gtk::InputPurpose::Url)
            .build();
        let dialog = adw::AlertDialog::builder()
            .heading("Add Feed")
            .body("The address of an RSS or Atom feed, or of a page of a site that publishes one.")
            .extra_child(&entry)
            .default_response("add")
            .close_response("cancel")
            .build();
        dialog.add_responses(&[("cancel", "_Cancel"), ("add", "_Add")]);
        dialog.set_response_appearance("add", adw::ResponseAppearance::Suggested);
        dialog.set_response_enabled("add", false);
        entry.connect_changed(glib::clone!(
            #[weak]
            dialog,
            move |entry| dialog.set_response_enabled("add", !entry.text().trim().is_empty())
        ));
        dialog.connect_response(
            Some("add"),
            glib::clone!(
                #[weak(rename_to = window)]
                self,
                #[weak]
                entry,
                move |_, _| window.follow_feed(entry.text().trim().to_owned())
            ),
        );
        dialog.present(Some(self));
        entry.grab_focus();
    }

    /// Finds the feed at `url` (a feed, or a page that advertises one) and follows it.
    pub fn follow_feed(&self, url: String) {
        let services = services::get();
        let (Some(rss), Some(store)) = (
            services.connector(&ServiceId::new("rss")),
            services.store.clone(),
        ) else {
            self.toast("Feeds cannot be followed in this build");
            return;
        };
        let task = runtime::spawn(async move {
            let found = rss.discover(&url).await?;
            let channel = found
                .into_iter()
                .chain(rss.parse_url(&url))
                .find_map(|target| match target {
                    Target::Channel(channel) => Some(channel),
                    Target::Item(_) | Target::Playlist(_) => None,
                })
                .ok_or(flaneur_connector::Error::NotFound)?;
            let channels = rss
                .channels()
                .ok_or(flaneur_connector::Error::Unsupported)?;
            let summary = channels.channel(&channel).await?.summary;
            store
                .lock()
                .add_local_subscription(&summary, services::now())
                .map_err(|e| flaneur_connector::Error::Service(e.to_string()))?;
            Ok::<_, flaneur_connector::Error>(summary.name)
        });
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            async move {
                match task.await {
                    Ok(Ok(name)) => {
                        window.toast(&format!("Following {name}"));
                        window.load_subscriptions();
                        window.show_section("subscriptions");
                    }
                    Ok(Err(flaneur_connector::Error::NotFound)) => {
                        window.toast("No feed found at this address");
                    }
                    Ok(Err(error)) => window.toast(&format!("Cannot add this feed: {error}")),
                    Err(error) => tracing::error!(%error, "adding a feed panicked"),
                }
            }
        ));
    }
}
