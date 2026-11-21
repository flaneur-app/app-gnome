// SPDX-License-Identifier: AGPL-3.0-or-later

//! The main window: a sidebar of sections beside their content, and the player on top.
//!
//! The navigation split view collapses into a single pane under 600sp, so the window fits a
//! phone-sized screen. The player is pushed on the outer navigation view and takes the whole
//! window.

use std::cell::OnceCell;

use adw::prelude::*;
use adw::subclass::prelude::*;
use flaneur_connector::FeedKind;
use flaneur_model::{Item, Playable};
use flaneur_vm::{Registry, SelectionPrefs, select_streams};
use gtk::{CompositeTemplate, gio, glib};

use crate::application::FlaneurApplication;
use crate::lists;
use crate::pages::{AccountsPage, FeedPage, SearchPage};
use crate::player::{self, Media, PlayerPage};
use crate::runtime;
use crate::widgets::VideoGrid;

mod imp {
    use super::{
        AccountsPage, CompositeTemplate, FeedPage, OnceCell, PlayerPage, Registry, SearchPage,
        SelectionPrefs, glib,
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
        pub registry: OnceCell<Registry>,
        pub selection_prefs: OnceCell<SelectionPrefs>,
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
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for FlaneurWindow {}
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
    /// A window showing the lists of the registry's connectors.
    pub fn new(app: &FlaneurApplication, registry: Registry) -> Self {
        let window: Self = glib::Object::builder().property("application", app).build();
        window.setup(registry);
        window
    }

    fn setup(&self, registry: Registry) {
        let imp = self.imp();
        if crate::config::is_devel() {
            self.add_css_class("devel");
        }

        imp.home_page.set_empty_description(
            "The home brings together the recommendations of every service you connect.",
        );
        imp.home_page
            .set_source(lists::feed(&registry, FeedKind::Home));
        imp.subscriptions_page.set_empty_description(
            "The latest videos of the channels you follow, on every service, in one feed.",
        );
        imp.subscriptions_page
            .set_source(lists::feed(&registry, FeedKind::Subscriptions));
        imp.history_page.set_empty_description(
            "What you watch, whatever the service, with the position to resume from.",
        );
        imp.history_page.set_source(lists::history(&registry));
        imp.search_page.set_registry(registry.clone());
        imp.accounts_page.set_registry(&registry);

        let grids: [VideoGrid; 4] = [
            imp.home_page.grid(),
            imp.subscriptions_page.grid(),
            imp.history_page.grid(),
            imp.search_page.grid(),
        ];
        for grid in grids {
            grid.connect_item_activated(glib::clone!(
                #[weak(rename_to = window)]
                self,
                move |_, object| window.play_item(object.item())
            ));
        }

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

        imp.registry
            .set(registry)
            .unwrap_or_else(|_| unreachable!("setup runs once"));
        self.show_section("home");
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
        imp.split_view.set_show_content(true);
        if name == "search" {
            imp.search_page.focus_entry();
        }
    }

    /// Shows a short message at the bottom of the window.
    pub fn toast(&self, message: &str) {
        self.imp().toast_overlay.add_toast(adw::Toast::new(message));
    }

    /// Resolves an item's streams with its connector, then plays it.
    fn play_item(&self, item: &Item) {
        let Some(connector) = self
            .imp()
            .registry
            .get()
            .and_then(|r| r.get(&item.r#ref.service))
            .cloned()
        else {
            self.toast("The service of this item is not available");
            return;
        };
        let item_ref = item.r#ref.clone();
        let task = runtime::spawn(async move { connector.player().resolve(&item_ref).await });
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            async move {
                match task.await {
                    Ok(Ok(playable)) => window.play_playable(&playable),
                    Ok(Err(error)) => window.toast(&format!("Cannot play this video: {error}")),
                    Err(error) => tracing::error!(%error, "stream resolution panicked"),
                }
            }
        ));
    }

    fn play_playable(&self, playable: &Playable) {
        let prefs = self.imp().selection_prefs.get_or_init(|| {
            let hardware_codecs = player::hardware_codecs();
            tracing::info!(?hardware_codecs, "hardware decoders");
            SelectionPrefs {
                hardware_codecs,
                ..SelectionPrefs::default()
            }
        });
        match select_streams(playable, prefs) {
            Some(selection) => self.play(&Media::from(selection), &playable.item.title),
            None => self.toast("This video has no stream Flâneur can play"),
        }
    }

    /// Opens the player on a media.
    pub fn play(&self, media: &Media, title: &str) {
        let imp = self.imp();
        imp.player_page.load(media, title);
        let player: &adw::NavigationPage = imp.player_page.upcast_ref();
        if imp.navigation.visible_page().as_ref() != Some(player) {
            imp.navigation.push(player);
        }
    }
}
