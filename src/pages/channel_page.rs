// SPDX-License-Identifier: AGPL-3.0-or-later

//! A channel page: header (avatar, name, followers, description) and the Videos tab.
//!
//! Channels whose service keeps follows in the local database (RSS feeds) get a Follow button.

use std::cell::{Cell, OnceCell};
use std::sync::Arc;

use adw::prelude::*;
use adw::subclass::prelude::*;
use flaneur_connector::Connector;
use flaneur_model::{Channel, ChannelSummary};
use gtk::{CompositeTemplate, glib};

use crate::services::{self, SharedStore};
use crate::widgets::VideoGrid;
use crate::{format, lists, runtime, thumbnails};

mod imp {
    use super::{Cell, ChannelSummary, CompositeTemplate, OnceCell, VideoGrid, glib};
    use adw::subclass::prelude::*;
    use gtk::prelude::*;

    #[derive(Default, CompositeTemplate)]
    #[template(resource = "/dev/baxyz/Flaneur/ui/channel-page.ui")]
    pub struct ChannelPage {
        #[template_child]
        pub follow_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub avatar: TemplateChild<adw::Avatar>,
        #[template_child]
        pub name: TemplateChild<gtk::Label>,
        #[template_child]
        pub details: TemplateChild<gtk::Label>,
        #[template_child]
        pub description: TemplateChild<gtk::Label>,
        #[template_child]
        pub grid: TemplateChild<VideoGrid>,
        pub summary: OnceCell<ChannelSummary>,
        pub following: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ChannelPage {
        const NAME: &'static str = "FlaneurChannelPage";
        type Type = super::ChannelPage;
        type ParentType = adw::NavigationPage;

        fn class_init(klass: &mut Self::Class) {
            VideoGrid::ensure_type();
            klass.bind_template();
            klass.bind_template_callbacks();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[gtk::template_callbacks]
    impl ChannelPage {
        #[template_callback]
        fn on_follow(&self) {
            self.obj().toggle_follow();
        }
    }

    impl ObjectImpl for ChannelPage {}
    impl WidgetImpl for ChannelPage {}
    impl NavigationPageImpl for ChannelPage {}
}

glib::wrapper! {
    /// A channel page.
    pub struct ChannelPage(ObjectSubclass<imp::ChannelPage>)
        @extends adw::NavigationPage, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl ChannelPage {
    /// A page for a channel; loads its header and videos in the background.
    pub fn new(summary: &ChannelSummary) -> Self {
        let page: Self = glib::Object::new();
        page.show_summary(summary);
        page.imp()
            .summary
            .set(summary.clone())
            .unwrap_or_else(|_| unreachable!("a new page has no channel yet"));

        let Some(connector) = services::get().connector(&summary.r#ref.service) else {
            return page;
        };
        let grid = &page.imp().grid;
        grid.set_empty_state("No Videos", None, None);
        grid.bind(&lists::channel_videos(
            Arc::clone(&connector),
            summary.r#ref.clone(),
        ));
        page.load_header(&connector);
        if lists::follows_locally(connector.as_ref())
            && let Some(store) = services::get().store.clone()
        {
            page.load_follow_state(store);
        }
        page
    }

    /// The grid, to react to activated items.
    pub fn grid(&self) -> VideoGrid {
        self.imp().grid.get()
    }

    fn show_summary(&self, summary: &ChannelSummary) {
        let imp = self.imp();
        self.set_title(&summary.name);
        imp.name.set_label(&summary.name);
        imp.avatar.set_text(Some(&summary.name));
        if let Some(url) = thumbnails::pick(&summary.avatars, 72) {
            let avatar = imp.avatar.downgrade();
            thumbnails::load(&url, move |texture| {
                if let Some(avatar) = avatar.upgrade() {
                    avatar.set_custom_image(Some(texture));
                }
            });
        }
    }

    fn load_header(&self, connector: &Arc<dyn Connector>) {
        if connector.channels().is_none() {
            return;
        }
        let Some(channel) = self.imp().summary.get().map(|s| s.r#ref.clone()) else {
            return;
        };
        let connector = Arc::clone(connector);
        let task = runtime::spawn(async move {
            let channels = connector
                .channels()
                .ok_or(flaneur_connector::Error::Unsupported)?;
            channels.channel(&channel).await
        });
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = page)]
            self,
            async move {
                match task.await {
                    Ok(Ok(channel)) => page.show_channel(&channel),
                    Ok(Err(error)) => tracing::warn!(%error, "channel header"),
                    Err(error) => tracing::warn!(%error, "channel header task"),
                }
            }
        ));
    }

    fn show_channel(&self, channel: &Channel) {
        let imp = self.imp();
        self.show_summary(&channel.summary);
        let details = [
            channel.handle.clone(),
            channel
                .subscribers
                .map(|n| format!("{} followers", format::compact(n))),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");
        imp.details.set_label(&details);
        imp.details.set_visible(!details.is_empty());
        let description = channel.description.as_deref().map_or("", str::trim);
        imp.description.set_label(description);
        imp.description.set_visible(!description.is_empty());
    }

    fn load_follow_state(&self, store: SharedStore) {
        let Some(summary) = self.imp().summary.get().cloned() else {
            return;
        };
        let task = runtime::spawn(async move {
            store
                .lock()
                .local_subscriptions(&summary.r#ref.service)
                .map(|followed| followed.iter().any(|c| c.r#ref == summary.r#ref))
        });
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = page)]
            self,
            async move {
                if let Ok(Ok(following)) = task.await {
                    page.set_following(following);
                    page.imp().follow_button.set_visible(true);
                }
            }
        ));
    }

    fn set_following(&self, following: bool) {
        let imp = self.imp();
        imp.following.set(following);
        imp.follow_button
            .set_label(if following { "Unfollow" } else { "Follow" });
        if following {
            imp.follow_button.remove_css_class("suggested-action");
        } else {
            imp.follow_button.add_css_class("suggested-action");
        }
    }

    fn toggle_follow(&self) {
        let imp = self.imp();
        let (Some(store), Some(summary)) =
            (services::get().store.clone(), imp.summary.get().cloned())
        else {
            return;
        };
        let follow = !imp.following.get();
        imp.follow_button.set_sensitive(false);
        let task = runtime::spawn(async move {
            let store = store.lock();
            if follow {
                store.add_local_subscription(&summary, services::now())
            } else {
                store.remove_local_subscription(&summary.r#ref)
            }
        });
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = page)]
            self,
            async move {
                page.imp().follow_button.set_sensitive(true);
                match task.await {
                    Ok(Ok(())) => {
                        page.set_following(follow);
                        let _ = page.activate_action("win.refresh-subscriptions", None);
                    }
                    Ok(Err(error)) => tracing::warn!(%error, "follow"),
                    Err(error) => tracing::warn!(%error, "follow task"),
                }
            }
        ));
    }
}
