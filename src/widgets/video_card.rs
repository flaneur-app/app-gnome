// SPDX-License-Identifier: AGPL-3.0-or-later

//! One item of a [`VideoGrid`](super::VideoGrid): thumbnail, service, title, channel, views and
//! date, and the watch progress when there is one.
//!
//! The channel button activates `win.show-channel` with the channel as `(service, id, name)`.

use std::cell::RefCell;

use flaneur_model::ItemKind;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{CompositeTemplate, glib};

use crate::item_object::ItemObject;
use crate::{format, services, thumbnails};

/// Width of a card's thumbnail, in logical pixels.
const THUMBNAIL_WIDTH: u32 = 240;

mod imp {
    use super::{CompositeTemplate, RefCell, glib};
    use gtk::subclass::prelude::*;

    #[derive(Default, CompositeTemplate)]
    #[template(resource = "/dev/baxyz/Flaneur/ui/video-card.ui")]
    pub struct VideoCard {
        #[template_child]
        pub thumbnail: TemplateChild<gtk::Picture>,
        #[template_child]
        pub placeholder: TemplateChild<gtk::Image>,
        #[template_child]
        pub service: TemplateChild<gtk::Label>,
        #[template_child]
        pub badge: TemplateChild<gtk::Label>,
        #[template_child]
        pub progress: TemplateChild<gtk::ProgressBar>,
        #[template_child]
        pub title: TemplateChild<gtk::Label>,
        #[template_child]
        pub channel: TemplateChild<gtk::Button>,
        #[template_child]
        pub channel_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub meta: TemplateChild<gtk::Label>,
        /// The thumbnail the card waits for; a late image of a previous item is ignored.
        pub thumbnail_url: RefCell<Option<String>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for VideoCard {
        const NAME: &'static str = "FlaneurVideoCard";
        type Type = super::VideoCard;
        type ParentType = gtk::Box;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for VideoCard {}
    impl WidgetImpl for VideoCard {}
    impl BoxImpl for VideoCard {}
}

glib::wrapper! {
    /// A video card.
    pub struct VideoCard(ObjectSubclass<imp::VideoCard>)
        @extends gtk::Box, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Orientable;
}

impl Default for VideoCard {
    fn default() -> Self {
        glib::Object::new()
    }
}

impl VideoCard {
    /// Shows an item.
    pub fn bind(&self, object: &ItemObject) {
        let imp = self.imp();
        let item = object.item();

        imp.title.set_label(&item.title);
        imp.title.set_tooltip_text(Some(&item.title));
        imp.service
            .set_label(&services::get().name(&item.r#ref.service));
        match &item.channel {
            Some(channel) => {
                imp.channel_label.set_label(&channel.name);
                imp.channel.set_action_target_value(Some(
                    &(
                        channel.r#ref.service.0.as_str(),
                        channel.r#ref.id.as_str(),
                        channel.name.as_str(),
                    )
                        .to_variant(),
                ));
                imp.channel.set_action_name(Some("win.show-channel"));
                imp.channel.set_visible(true);
            }
            None => imp.channel.set_visible(false),
        }
        imp.meta.set_label(&format::meta(item, services::now()));

        let badge = format::badge(item);
        imp.badge.set_visible(badge.is_some());
        imp.badge.set_label(badge.as_deref().unwrap_or_default());
        if item.kind == ItemKind::Live {
            imp.badge.add_css_class("live");
        } else {
            imp.badge.remove_css_class("live");
        }

        let fraction = item.watch.and_then(|w| w.progress).and_then(|p| {
            let duration = p.duration.filter(|d| *d > 0)?;
            Some(f64::from(p.position.min(duration)) / f64::from(duration))
        });
        imp.progress.set_visible(fraction.is_some());
        imp.progress.set_fraction(fraction.unwrap_or_default());

        imp.placeholder.set_icon_name(Some(match item.kind {
            ItemKind::Live | ItemKind::Premiere | ItemKind::Upcoming => "camera-web-symbolic",
            ItemKind::Video | ItemKind::Short => "video-x-generic-symbolic",
        }));
        imp.placeholder.set_visible(true);
        imp.thumbnail.set_paintable(gtk::gdk::Paintable::NONE);
        let url = thumbnails::pick(&item.thumbnails, THUMBNAIL_WIDTH);
        imp.thumbnail_url.replace(url.clone());
        if let Some(url) = url {
            let card = self.downgrade();
            let wanted = url.clone();
            thumbnails::load(&url, move |texture| {
                let Some(card) = card.upgrade() else { return };
                let imp = card.imp();
                if imp.thumbnail_url.borrow().as_deref() == Some(wanted.as_str()) {
                    imp.thumbnail.set_paintable(Some(texture));
                    imp.placeholder.set_visible(false);
                }
            });
        }
    }
}
