// SPDX-License-Identifier: AGPL-3.0-or-later

//! One item of a [`VideoGrid`](super::VideoGrid): thumbnail, title, channel, views and date.

use std::time::{SystemTime, UNIX_EPOCH};

use flaneur_model::ItemKind;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{CompositeTemplate, glib};

use crate::format;
use crate::item_object::ItemObject;

mod imp {
    use super::{CompositeTemplate, glib};
    use gtk::subclass::prelude::*;

    #[derive(Default, CompositeTemplate)]
    #[template(resource = "/dev/baxyz/Flaneur/ui/video-card.ui")]
    pub struct VideoCard {
        #[template_child]
        pub placeholder: TemplateChild<gtk::Image>,
        #[template_child]
        pub badge: TemplateChild<gtk::Label>,
        #[template_child]
        pub title: TemplateChild<gtk::Label>,
        #[template_child]
        pub channel: TemplateChild<gtk::Label>,
        #[template_child]
        pub meta: TemplateChild<gtk::Label>,
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
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .and_then(|d| i64::try_from(d.as_secs()).ok())
            .unwrap_or_default();

        imp.title.set_label(&item.title);
        imp.title.set_tooltip_text(Some(&item.title));
        let channel = item.channel.as_ref().map(|c| c.name.as_str());
        imp.channel.set_label(channel.unwrap_or_default());
        imp.channel.set_visible(channel.is_some());
        imp.meta.set_label(&format::meta(item, now));

        let badge = format::badge(item);
        imp.badge.set_visible(badge.is_some());
        imp.badge.set_label(badge.as_deref().unwrap_or_default());
        imp.placeholder.set_icon_name(Some(match item.kind {
            ItemKind::Live | ItemKind::Premiere | ItemKind::Upcoming => "camera-web-symbolic",
            ItemKind::Video | ItemKind::Short => "video-x-generic-symbolic",
        }));
        if item.kind == ItemKind::Live {
            imp.badge.add_css_class("live");
        } else {
            imp.badge.remove_css_class("live");
        }
    }
}
