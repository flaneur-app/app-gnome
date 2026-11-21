// SPDX-License-Identifier: AGPL-3.0-or-later

//! The application: single instance, command line, app-wide actions.
//!
//! `flaneur --play URI` opens the player on any URI GStreamer can read (or a file path), and
//! `--audio URI` adds a separate audio stream played in sync, the way split streams of
//! services play. A second invocation is forwarded to the running instance.

use adw::prelude::*;
use gtk::{gio, glib};

use crate::config;
use crate::player::Media;
use crate::window::FlaneurWindow;

mod imp {
    use super::{FlaneurWindow, gio, glib};
    use adw::prelude::*;
    use adw::subclass::prelude::*;

    #[derive(Default)]
    pub struct FlaneurApplication {}

    #[glib::object_subclass]
    impl ObjectSubclass for FlaneurApplication {
        const NAME: &'static str = "FlaneurApplication";
        type Type = super::FlaneurApplication;
        type ParentType = adw::Application;
    }

    impl ObjectImpl for FlaneurApplication {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();
            obj.add_main_option(
                "play",
                glib::Char::from(b'p'),
                glib::OptionFlags::NONE,
                glib::OptionArg::String,
                "Play a video URI or file",
                Some("URI"),
            );
            obj.add_main_option(
                "audio",
                glib::Char::from(b'a'),
                glib::OptionFlags::NONE,
                glib::OptionArg::String,
                "With --play, a separate audio stream to play in sync",
                Some("URI"),
            );
            obj.setup_actions();
        }
    }

    impl ApplicationImpl for FlaneurApplication {
        // Runs in the invoking process, before the command line goes to the primary instance.
        fn handle_local_options(
            &self,
            options: &glib::VariantDict,
        ) -> std::ops::ControlFlow<glib::ExitCode> {
            if options.contains("audio") && !options.contains("play") {
                eprintln!("flaneur: --audio needs --play");
                return std::ops::ControlFlow::Break(glib::ExitCode::FAILURE);
            }
            self.parent_handle_local_options(options)
        }

        fn startup(&self) {
            self.parent_startup();
            gtk::Window::set_default_icon_name(crate::config::APP_ID);
        }

        fn activate(&self) {
            let app = self.obj();
            if let Some(window) = app.active_window() {
                window.present();
                return;
            }
            FlaneurWindow::new(&app).present();
        }

        fn command_line(&self, command_line: &gio::ApplicationCommandLine) -> glib::ExitCode {
            self.obj().handle_command_line(command_line)
        }
    }

    impl GtkApplicationImpl for FlaneurApplication {}
    impl AdwApplicationImpl for FlaneurApplication {}
}

glib::wrapper! {
    /// The Flâneur application.
    pub struct FlaneurApplication(ObjectSubclass<imp::FlaneurApplication>)
        @extends adw::Application, gtk::Application, gio::Application,
        @implements gio::ActionGroup, gio::ActionMap;
}

impl Default for FlaneurApplication {
    fn default() -> Self {
        glib::Object::builder()
            .property("application-id", config::APP_ID)
            .property("flags", gio::ApplicationFlags::HANDLES_COMMAND_LINE)
            .property("resource-base-path", config::RESOURCE_BASE_PATH)
            .build()
    }
}

impl FlaneurApplication {
    fn setup_actions(&self) {
        let quit = gio::ActionEntry::builder("quit")
            .activate(|app: &Self, _, _| app.quit())
            .build();
        let about = gio::ActionEntry::builder("about")
            .activate(|app: &Self, _, _| app.show_about())
            .build();
        self.add_action_entries([quit, about]);
        self.set_accels_for_action("app.quit", &["<Control>q"]);
        self.set_accels_for_action("window.close", &["<Control>w"]);
    }

    fn handle_command_line(&self, command_line: &gio::ApplicationCommandLine) -> glib::ExitCode {
        let options = command_line.options_dict();
        let uri = |name: &str| {
            options
                .lookup::<String>(name)
                .ok()
                .flatten()
                .map(|arg| to_uri(command_line, &arg))
        };
        let (video, audio) = (uri("play"), uri("audio"));

        self.activate();
        if let Some(video) = video {
            let title = title_of(&video);
            let media = match audio {
                Some(audio) => Media::Split { video, audio },
                None => Media::Uri(video),
            };
            if let Some(window) = self.active_window().and_downcast::<FlaneurWindow>() {
                window.play_uri(&media, &title);
            }
        }
        glib::ExitCode::SUCCESS
    }

    fn show_about(&self) {
        let about = adw::AboutDialog::builder()
            .application_name("Flâneur")
            .application_icon(config::APP_ID)
            .developer_name("The Flâneur contributors")
            .version(config::VERSION)
            .website(config::WEBSITE)
            .issue_url(config::ISSUE_URL)
            .license_type(gtk::License::Agpl30)
            .copyright("© 2026 The Flâneur contributors")
            .comments(
                "Wander your favorite video platforms from one fast, open app.\n\n\
                 Flâneur is an independent project. It is not affiliated with, endorsed by or \
                 sponsored by any of the services it connects to; their names and trademarks \
                 belong to their owners.",
            )
            .build();
        about.present(self.active_window().as_ref());
    }
}

/// A URI as is, or a file path (relative to the caller's directory) as a `file://` URI.
fn to_uri(command_line: &gio::ApplicationCommandLine, arg: &str) -> String {
    if glib::Uri::peek_scheme(arg).is_some() {
        arg.to_owned()
    } else {
        command_line.create_file_for_arg(arg).uri().into()
    }
}

/// The last path segment of a URI, as the player title.
fn title_of(uri: &str) -> String {
    url::Url::parse(uri)
        .ok()
        .and_then(|url| {
            url.path_segments()
                .and_then(|mut segments| segments.next_back().map(str::to_owned))
        })
        .filter(|segment| !segment.is_empty())
        .unwrap_or_else(|| uri.to_owned())
}

#[cfg(test)]
mod tests {
    use super::title_of;

    #[test]
    fn titles_come_from_the_last_path_segment() {
        assert_eq!(
            title_of("https://example.org/videos/trailer.mp4"),
            "trailer.mp4"
        );
        assert_eq!(title_of("file:///home/me/clip.webm"), "clip.webm");
        assert_eq!(title_of("https://example.org/"), "https://example.org/");
    }
}
