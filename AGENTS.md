# AGENTS.md — Flâneur `app-gnome`

This repository inherits the [canonical workspace rules](https://github.com/flaneur-app/.dev/blob/main/AGENTS.md). Only project-specific details are documented here.

## Scope

The GNOME app of [Flâneur](https://github.com/flaneur-app): a native GTK4/libadwaita client, packaged as a Flatpak. Screens, state and logic come from [`flaneur-app/core`](https://github.com/flaneur-app/core); this repository only holds the native GNOME interface.

**Status:** bootstrapping. Whether this app starts as a fork of [Pipeline](https://gitlab.com/schmiddi-on-mobile/pipeline) is being evaluated.

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

## Commands

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo nextest run --workspace --no-tests=pass
cargo deny check
```
