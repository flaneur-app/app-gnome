// SPDX-License-Identifier: AGPL-3.0-or-later

//! [`flaneur_model::Item`] as a GObject, for `gio::ListStore` and list views.

use std::cell::OnceCell;

use flaneur_model::Item;
use gtk::glib;
use gtk::subclass::prelude::*;

mod imp {
    use super::{Item, ObjectImpl, ObjectSubclass, OnceCell, glib};

    #[derive(Default)]
    pub struct ItemObject {
        pub item: OnceCell<Item>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ItemObject {
        const NAME: &'static str = "FlaneurItem";
        type Type = super::ItemObject;
    }

    impl ObjectImpl for ItemObject {}
}

glib::wrapper! {
    /// An immutable item of a list model.
    pub struct ItemObject(ObjectSubclass<imp::ItemObject>);
}

impl ItemObject {
    /// Wraps an item.
    pub fn new(item: Item) -> Self {
        let object: Self = glib::Object::new();
        object
            .imp()
            .item
            .set(item)
            .unwrap_or_else(|_| unreachable!("a new object has no item yet"));
        object
    }

    /// The wrapped item.
    pub fn item(&self) -> &Item {
        self.imp().item.get().expect("set by ItemObject::new")
    }
}
