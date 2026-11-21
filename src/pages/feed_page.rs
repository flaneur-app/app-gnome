// SPDX-License-Identifier: AGPL-3.0-or-later

//! A feed page: home, subscriptions or history.

use gtk::subclass::prelude::*;
use gtk::{CompositeTemplate, glib};

use crate::lists::ListBinding;
use crate::widgets::VideoGrid;

mod imp {
    use super::{CompositeTemplate, VideoGrid, glib};
    use adw::subclass::prelude::*;
    use gtk::prelude::*;

    #[derive(Default, CompositeTemplate)]
    #[template(resource = "/dev/baxyz/Flaneur/ui/feed-page.ui")]
    pub struct FeedPage {
        #[template_child]
        pub stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub no_service_status: TemplateChild<adw::StatusPage>,
        #[template_child]
        pub grid: TemplateChild<VideoGrid>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for FeedPage {
        const NAME: &'static str = "FlaneurFeedPage";
        type Type = super::FeedPage;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            VideoGrid::ensure_type();
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for FeedPage {}
    impl WidgetImpl for FeedPage {}
    impl BinImpl for FeedPage {}
}

glib::wrapper! {
    /// A feed page.
    pub struct FeedPage(ObjectSubclass<imp::FeedPage>)
        @extends adw::Bin, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl FeedPage {
    /// Sets what the empty state says: what the page shows once a service provides it.
    pub fn set_empty_description(&self, description: &str) {
        self.imp()
            .no_service_status
            .set_description(Some(description));
    }

    /// Shows a list, or the empty state when no connected service provides one.
    pub fn set_source(&self, source: Option<ListBinding>) {
        let imp = self.imp();
        if let Some(binding) = source {
            imp.grid.bind(&binding);
            imp.stack.set_visible_child_name("items");
        } else {
            imp.stack.set_visible_child_name("no-service");
        }
    }

    /// The grid, to react to activated items.
    pub fn grid(&self) -> VideoGrid {
        self.imp().grid.get()
    }
}
