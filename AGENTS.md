# AGENTS.md — Flâneur `app-gnome`

This repository inherits the [canonical workspace rules](https://github.com/flaneur-app/.dev/blob/main/AGENTS.md). Only project-specific details are documented here.

## Scope

The GNOME app of [Flâneur](https://github.com/flaneur-app): a native GTK4/libadwaita client, packaged as a Flatpak. Screens, state and logic come from [`flaneur-app/core`](https://github.com/flaneur-app/core); this repository only holds the native GNOME interface.

**Status:** early development: app shell, video grid and player; no connector registered yet.

This app is **not** a fork of [Pipeline](https://gitlab.com/schmiddi-on-mobile/pipeline) (see the
Pipeline spike in `flaneur-app/spikes`). Pipeline may inspire layouts and behaviors, but never copy
its code: it is GPL-3.0 and its maintainers refuse LLM-generated contributions.

## License

**AGPL-3.0-or-later**. Every source file starts with an SPDX header:

```rust
// SPDX-License-Identifier: AGPL-3.0-or-later
```

Dependencies must have AGPL-compatible licenses; `cargo deny check` enforces the list in `deny.toml`.

## Naming and legal positioning

- Never use "YouTube" or "tube" in the name of the app, a crate meant for end users, or the org.
  Service names appear only where they describe what a connector talks to.
- No ad blocking and no downloading as advertised features.
- Cookies and tokens stay on the user's machine (OS keyring). No code path sends them anywhere but
  to the service they belong to.
- Every README keeps the non-affiliation notice.

## Commit Scopes

Defined in `scopes.json`:

| Scope | Covers |
| --- | --- |
| `ui` | Windows, pages and widgets (GTK4/libadwaita) |
| `player` | Video playback integration |
| `flatpak` | Flatpak manifest, AppStream metadata, desktop file |
| `i18n` | Translations |
| `build`, `test`, `docs`, `deps`, `ci`, `ai` | As usual |

## Structure

```text
Cargo.toml              binary crate `flaneur`; core crates from git (flaneur-app/core)
build.rs                compiles data/ui/*.blp (Blueprint) and bundles data/ into a GResource
data/
  ui/*.blp              composite templates of the window, pages and widgets
  style.css             app stylesheet, loaded automatically from the resource base path
  icons/                app icon (scalable) and its symbolic variant
  dev.baxyz.Flaneur.desktop, dev.baxyz.Flaneur.metainfo.xml
build-aux/
  dev.baxyz.Flaneur.Devel.yml   Flatpak manifest (GNOME 51, built in CI)
  install.sh            installs the binary and data files under a prefix, for an app id
src/
  main.rs               logging, resources, GStreamer init, run
  application.rs        AdwApplication: command line (--play, --audio), about dialog, actions
  window.rs             main window: navigation split view, sections, player navigation
  services.rs           the connectors of this build (register new ones here); resolve()
  lists.rs              ListBinding (a flaneur_vm::Paginator observed from GTK) and sources
  runtime.rs            the tokio runtime: network work never runs on the GTK thread
  pages/                feed page (home, subscriptions, history), search, accounts
  widgets/              VideoGrid (GtkGridView over a ListStore of ItemObject), VideoCard
  player/               Engine (playbin3 + gtk4paintablesink), PlayerPage, Media, decoders
  item_object.rs        flaneur_model::Item as a GObject
  format.rs             durations, counts and dates for display
  demo.rs               fake connector for tests and the `demo` feature
```

Threading: widgets live on the GTK main thread. Work for connectors goes to the tokio runtime
(`runtime::spawn`); results come back by awaiting the `JoinHandle` or a `watch::Receiver` inside
`glib::spawn_future_local`. Never block the main thread on the network.

UI: Blueprint templates for layout, Rust for behavior. Native builds target libadwaita 1.7 and
GTK 4.18 (Debian trixie, the dev container): the crate features stop at `v1_7` and `v4_18`, even
though the Flatpak runtime is newer, so the app keeps building and running in the container.

## Commands

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo nextest run --workspace --no-tests=pass
cargo deny check
appstreamcli validate --no-net data/dev.baxyz.Flaneur.metainfo.xml
desktop-file-validate data/dev.baxyz.Flaneur.desktop
```

Run and inspect (the container has no display):

```bash
cargo run --features demo -- --play URI          # demo service, or play a URI directly
gtk4-broadwayd --port 8080 :5 &                  # then GDK_BACKEND=broadway BROADWAY_DISPLAY=:5
G_DEBUG=fatal-criticals RUST_LOG=flaneur=debug   # fail on GTK criticals, log player states
```

For screenshots, Xvfb is more reliable than Broadway with a headless browser:
`Xvfb :9 &`, then `DISPLAY=:9 GDK_BACKEND=x11 dbus-run-session -- cargo run`, and
`DISPLAY=:9 xwd -root | convert xwd:- shot.png` (packages `xvfb x11-apps imagemagick`).

The Flatpak builds only in CI (`flatpak` job): `flatpak-builder` cannot run in the dev container.
