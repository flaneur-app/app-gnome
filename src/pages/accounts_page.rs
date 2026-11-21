// SPDX-License-Identifier: AGPL-3.0-or-later

//! The accounts page: the services of this build and their session state.
//!
//! Signing in comes in a later version; sessions will live in the system keyring (the Secret
//! portal in the Flatpak), never in a file of the app. Services without accounts (RSS) keep
//! their follows in the local database, and their row offers to add a feed.

use adw::prelude::*;
use flaneur_vm::Registry;
use gtk::subclass::prelude::*;
use gtk::{CompositeTemplate, glib};

mod imp {
    use super::{CompositeTemplate, glib};
    use adw::subclass::prelude::*;

    #[derive(Default, CompositeTemplate)]
    #[template(resource = "/dev/baxyz/Flaneur/ui/accounts-page.ui")]
    pub struct AccountsPage {
        #[template_child]
        pub stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub services_group: TemplateChild<adw::PreferencesGroup>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for AccountsPage {
        const NAME: &'static str = "FlaneurAccountsPage";
        type Type = super::AccountsPage;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for AccountsPage {}
    impl WidgetImpl for AccountsPage {}
    impl BinImpl for AccountsPage {}
}

glib::wrapper! {
    /// The accounts page.
    pub struct AccountsPage(ObjectSubclass<imp::AccountsPage>)
        @extends adw::Bin, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl AccountsPage {
    /// Lists the connectors of the registry.
    pub fn set_registry(&self, registry: &Registry) {
        let imp = self.imp();
        let mut any = false;
        for connector in registry.iter() {
            any = true;
            let info = connector.info();
            let status = if info.auth_methods.is_empty() {
                "No account needed: follows stay on this computer"
            } else if connector.capabilities().signed_in {
                "Signed in"
            } else {
                "Works without an account; signing in comes in a later version"
            };
            let row = adw::ActionRow::builder()
                .title(info.name.as_str())
                .subtitle(status)
                .use_markup(false)
                .build();
            if crate::lists::follows_locally(connector.as_ref()) {
                let add = gtk::Button::builder()
                    .label("_Add Feed")
                    .use_underline(true)
                    .valign(gtk::Align::Center)
                    .action_name("win.add-feed")
                    .build();
                row.add_suffix(&add);
            }
            imp.services_group.add(&row);
        }
        imp.stack
            .set_visible_child_name(if any { "services" } else { "empty" });
    }
}
