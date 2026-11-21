// SPDX-License-Identifier: AGPL-3.0-or-later

//! The search page: an entry and the grid of results.

use std::cell::OnceCell;

use flaneur_vm::Registry;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{CompositeTemplate, glib};

use crate::lists;
use crate::widgets::VideoGrid;

mod imp {
    use super::{CompositeTemplate, OnceCell, Registry, VideoGrid, glib};
    use adw::subclass::prelude::*;
    use gtk::prelude::*;

    #[derive(Default, CompositeTemplate)]
    #[template(resource = "/dev/baxyz/Flaneur/ui/search-page.ui")]
    pub struct SearchPage {
        #[template_child]
        pub entry: TemplateChild<gtk::SearchEntry>,
        #[template_child]
        pub stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub grid: TemplateChild<VideoGrid>,
        pub registry: OnceCell<Registry>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SearchPage {
        const NAME: &'static str = "FlaneurSearchPage";
        type Type = super::SearchPage;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            VideoGrid::ensure_type();
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for SearchPage {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();
            self.entry.connect_activate(glib::clone!(
                #[weak]
                obj,
                move |entry| obj.search(&entry.text())
            ));
        }
    }

    impl WidgetImpl for SearchPage {}
    impl BinImpl for SearchPage {}
}

glib::wrapper! {
    /// The search page.
    pub struct SearchPage(ObjectSubclass<imp::SearchPage>)
        @extends adw::Bin, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl SearchPage {
    /// Sets the connectors to search; called once by the window.
    pub fn set_registry(&self, registry: Registry) {
        let imp = self.imp();
        let searchable = lists::can_search(&registry);
        imp.entry.set_sensitive(searchable);
        imp.stack
            .set_visible_child_name(if searchable { "start" } else { "no-service" });
        imp.registry
            .set(registry)
            .unwrap_or_else(|_| panic!("SearchPage::set_registry is called once"));
    }

    /// Focuses the entry.
    pub fn focus_entry(&self) {
        self.imp().entry.grab_focus();
    }

    /// The grid, to react to activated items.
    pub fn grid(&self) -> VideoGrid {
        self.imp().grid.get()
    }

    fn search(&self, query: &str) {
        let imp = self.imp();
        let query = query.trim();
        if query.is_empty() {
            return;
        }
        let Some(binding) = imp.registry.get().and_then(|r| lists::search(r, query)) else {
            return;
        };
        imp.grid.bind(&binding);
        imp.stack.set_visible_child_name("results");
    }
}
