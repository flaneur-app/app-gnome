// SPDX-License-Identifier: AGPL-3.0-or-later

//! Flâneur for GNOME: a fast, open, multi-service video client.
//!
//! The screens are GTK 4 and libadwaita widgets over the view-models of `flaneur-vm`; network
//! work runs on a tokio runtime of its own (see [`runtime`]), playback on GStreamer (see
//! [`player`]).

mod application;
mod config;
#[cfg(any(test, feature = "demo"))]
mod demo;
mod format;
mod item_object;
mod lists;
mod pages;
mod player;
mod runtime;
mod services;
mod thumbnails;
mod widgets;
mod window;

use std::io::IsTerminal;

use gtk::prelude::*;
use gtk::{gio, glib};
use tracing_subscriber::EnvFilter;

use application::FlaneurApplication;

fn main() -> glib::ExitCode {
    // `RUST_LOG=flaneur=debug` for more; GStreamer has its own `GST_DEBUG`.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_ansi(std::io::stderr().is_terminal())
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("flaneur=info")),
        )
        .init();

    // The connectors bring rustls with two crypto providers (ring and aws-lc-rs), so rustls
    // cannot pick one for clients built without an explicit provider (RustyPipe's, the
    // thumbnails'): install ring, which every connector supports, for the whole process.
    if rustls::crypto::ring::default_provider()
        .install_default()
        .is_err()
    {
        tracing::debug!("a rustls crypto provider was already installed");
    }

    gio::resources_register_include!("flaneur.gresource").expect("the bundled resources are valid");
    if let Err(error) = gst::init() {
        tracing::error!(%error, "GStreamer failed to initialize");
        return glib::ExitCode::FAILURE;
    }
    glib::set_application_name("Flâneur");

    FlaneurApplication::default().run()
}
