// SPDX-License-Identifier: AGPL-3.0-or-later

//! A grid of video cards bound to a paginated list.
//!
//! The grid observes a [`ListBinding`]: a task on the GTK main loop awaits the paginator's
//! `watch` channel and mirrors each new state into a `gio::ListStore` of [`ItemObject`]s, so
//! widgets are only touched on the main thread while pages load on the tokio runtime. Coming
//! within one screen of the end loads the next page. Activating a card emits `item-activated`.

use std::cell::RefCell;
use std::sync::OnceLock;

use flaneur_model::Item;
use flaneur_vm::ListState;
use gtk::glib::subclass::Signal;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{CompositeTemplate, gio, glib};

use super::VideoCard;
use crate::item_object::ItemObject;
use crate::lists::ListBinding;

mod imp {
    use super::{
        CompositeTemplate, ItemObject, ListBinding, OnceLock, RefCell, Signal, StaticType,
        VideoCard, gio, glib,
    };
    use adw::subclass::prelude::*;
    use gtk::prelude::*;

    #[derive(CompositeTemplate)]
    #[template(resource = "/dev/baxyz/Flaneur/ui/video-grid.ui")]
    pub struct VideoGrid {
        #[template_child]
        pub banner: TemplateChild<adw::Banner>,
        #[template_child]
        pub stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub error_status: TemplateChild<adw::StatusPage>,
        #[template_child]
        pub scrolled: TemplateChild<gtk::ScrolledWindow>,
        #[template_child]
        pub grid_view: TemplateChild<gtk::GridView>,
        #[template_child]
        pub more_revealer: TemplateChild<gtk::Revealer>,
        pub store: gio::ListStore,
        pub binding: RefCell<Option<ListBinding>>,
        pub observer: RefCell<Option<glib::JoinHandle<()>>>,
    }

    impl Default for VideoGrid {
        fn default() -> Self {
            Self {
                banner: TemplateChild::default(),
                stack: TemplateChild::default(),
                error_status: TemplateChild::default(),
                scrolled: TemplateChild::default(),
                grid_view: TemplateChild::default(),
                more_revealer: TemplateChild::default(),
                store: gio::ListStore::new::<ItemObject>(),
                binding: RefCell::default(),
                observer: RefCell::default(),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for VideoGrid {
        const NAME: &'static str = "FlaneurVideoGrid";
        type Type = super::VideoGrid;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
            klass.bind_template_callbacks();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[gtk::template_callbacks]
    impl VideoGrid {
        #[template_callback]
        fn on_retry(&self) {
            self.obj().retry();
        }
    }

    impl ObjectImpl for VideoGrid {
        fn signals() -> &'static [Signal] {
            static SIGNALS: OnceLock<Vec<Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| {
                vec![
                    Signal::builder("item-activated")
                        .param_types([ItemObject::static_type()])
                        .build(),
                ]
            })
        }

        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();

            let factory = gtk::SignalListItemFactory::new();
            factory.connect_setup(|_, list_item| {
                let list_item = list_item
                    .downcast_ref::<gtk::ListItem>()
                    .expect("a grid view item is a ListItem");
                list_item.set_child(Some(&VideoCard::default()));
            });
            factory.connect_bind(|_, list_item| {
                let list_item = list_item
                    .downcast_ref::<gtk::ListItem>()
                    .expect("a grid view item is a ListItem");
                if let (Some(card), Some(object)) = (
                    list_item.child().and_downcast::<VideoCard>(),
                    list_item.item().and_downcast::<ItemObject>(),
                ) {
                    card.bind(&object);
                }
            });
            self.grid_view.set_factory(Some(&factory));
            self.grid_view
                .set_model(Some(&gtk::NoSelection::new(Some(self.store.clone()))));

            self.grid_view.connect_activate(glib::clone!(
                #[weak]
                obj,
                move |_, position| {
                    if let Some(object) =
                        obj.imp().store.item(position).and_downcast::<ItemObject>()
                    {
                        obj.emit_by_name::<()>("item-activated", &[&object]);
                    }
                }
            ));
            // Loads the next page when the end of the list comes within one screen, and when
            // the loaded items do not fill the view yet.
            let adjustment = self.scrolled.vadjustment();
            adjustment.connect_value_changed(glib::clone!(
                #[weak]
                obj,
                move |_| obj.load_more_if_near_the_end()
            ));
            adjustment.connect_changed(glib::clone!(
                #[weak]
                obj,
                move |_| obj.load_more_if_near_the_end()
            ));
            self.banner.connect_button_clicked(glib::clone!(
                #[weak]
                obj,
                move |_| obj.retry()
            ));
        }

        fn dispose(&self) {
            if let Some(observer) = self.observer.take() {
                observer.abort();
            }
        }
    }

    impl WidgetImpl for VideoGrid {}
    impl BinImpl for VideoGrid {}
}

glib::wrapper! {
    /// A grid of video cards over a paginated list.
    pub struct VideoGrid(ObjectSubclass<imp::VideoGrid>)
        @extends adw::Bin, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl VideoGrid {
    /// Shows a list, replacing the previous one, and loads its first page if nothing is loaded.
    pub fn bind(&self, binding: &ListBinding) {
        let imp = self.imp();
        if let Some(previous) = imp.observer.take() {
            previous.abort();
        }
        imp.store.remove_all();

        let mut state = binding.state();
        let initial = state.borrow().clone();
        imp.binding.replace(Some(binding.clone()));

        let weak = self.downgrade();
        let observer = glib::spawn_future_local(async move {
            loop {
                let current = state.borrow_and_update().clone();
                let Some(grid) = weak.upgrade() else { break };
                grid.apply(&current);
                drop(grid);
                if state.changed().await.is_err() {
                    break;
                }
            }
        });
        imp.observer.replace(Some(observer));

        if initial.items.is_empty() && !initial.loading && initial.error.is_none() {
            binding.load_more();
        }
    }

    /// Connects to `item-activated`.
    pub fn connect_item_activated<F: Fn(&Self, &ItemObject) + 'static>(
        &self,
        f: F,
    ) -> glib::SignalHandlerId {
        self.connect_closure(
            "item-activated",
            false,
            glib::closure_local!(move |grid: &Self, object: &ItemObject| f(grid, object)),
        )
    }

    fn load_more_if_near_the_end(&self) {
        let imp = self.imp();
        let binding = imp.binding.borrow();
        let Some(binding) = binding.as_ref() else {
            return;
        };
        let ready = {
            let state = binding.state();
            let state = state.borrow();
            !state.items.is_empty() && state.has_more && !state.loading && state.error.is_none()
        };
        let adjustment = imp.scrolled.vadjustment();
        let remaining = adjustment.upper() - adjustment.value() - adjustment.page_size();
        if ready && remaining < adjustment.page_size() {
            binding.load_more();
        }
    }

    fn retry(&self) {
        if let Some(binding) = self.imp().binding.borrow().as_ref() {
            binding.load_more();
        }
    }

    /// Mirrors a list state into the widgets.
    fn apply(&self, state: &ListState<Item>) {
        let imp = self.imp();
        let store = &imp.store;
        let shown = usize::try_from(store.n_items()).unwrap_or(usize::MAX);
        let is_prefix = shown <= state.items.len()
            && (0..shown).all(|i| {
                u32::try_from(i)
                    .ok()
                    .and_then(|i| store.item(i))
                    .and_downcast::<ItemObject>()
                    .is_some_and(|o| o.item().r#ref == state.items[i].r#ref)
            });
        let start = if is_prefix { shown } else { 0 };
        let added: Vec<ItemObject> = state.items[start..]
            .iter()
            .cloned()
            .map(ItemObject::new)
            .collect();
        if is_prefix {
            store.extend_from_slice(&added);
        } else {
            store.splice(0, store.n_items(), &added);
        }

        let empty = state.items.is_empty();
        let page = match (&state.error, empty) {
            (_, false) => "items",
            (Some(_), true) => "error",
            (None, true) if state.loading || state.has_more => "loading",
            (None, true) => "empty",
        };
        tracing::trace!(
            items = state.items.len(),
            loading = state.loading,
            page,
            "grid state"
        );
        imp.stack.set_visible_child_name(page);
        imp.error_status
            .set_description(state.error.as_deref().filter(|_| empty));
        imp.banner
            .set_title(state.error.as_deref().unwrap_or_default());
        imp.banner.set_revealed(state.error.is_some() && !empty);
        imp.more_revealer.set_reveal_child(state.loading && !empty);
    }
}
