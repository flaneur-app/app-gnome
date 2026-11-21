# Flâneur for GNOME

The GNOME app of [Flâneur](https://github.com/flaneur-app): a native GTK4/libadwaita client, packaged as a Flatpak. Screens, state and logic come from [`flaneur-app/core`](https://github.com/flaneur-app/core); this repository only holds the native GNOME interface.

> Flâneur is an independent, open-source project. It is not affiliated with, endorsed by or
> sponsored by Google, YouTube, Odysee, LBRY or any other service it can connect to. All
> trademarks belong to their owners.

**Status:** early development, without accounts. What works:

- **Home**: the home feeds of the services that have one without an account (Odysee today),
  interleaved, each card showing its service.
- **Search** across YouTube and Odysee, results interleaved.
- **Subscriptions**: RSS and Atom feeds you follow (Add Feed takes a feed or a page that
  advertises one), merged by date. Following YouTube and Odysee channels needs sign-in, which
  comes later.
- **History**: every video played in Flâneur, with resume positions, kept on this computer.
- **Channel pages** from a card's channel name, with Follow for RSS feeds.
- **Player**: the best streams the GPU can decode (1080p at most), resume, related items under
  the video, autoplay of the next one, fullscreen, keyboard shortcuts.

Known limits are listed in the pull requests; `flaneur --play <uri>` plays any video URI.

## Build and run in the dev container

The [dev container](https://github.com/flaneur-app/.dev) has Rust, GTK 4.18, libadwaita 1.7,
GStreamer 1.26 and `blueprint-compiler`. The player also needs the GStreamer runtime plugins:

```bash
sudo apt-get install -y gstreamer1.0-gtk4 gstreamer1.0-plugins-good gstreamer1.0-plugins-bad \
  gstreamer1.0-libav
```

```bash
cargo run                                          # the app
cargo run --features demo                          # with a fake "Demo" service filling the grid
cargo run -- --play https://download.blender.org/durian/trailer/sintel_trailer-480p.mp4
cargo run -- --play VIDEO_URI --audio AUDIO_URI    # separate video and audio streams, in sync
```

`--play` also takes a file path. `RUST_LOG=flaneur=debug` logs more (player states, selected
streams, hardware decoders); `GST_DEBUG=3` turns on GStreamer's own warnings.

The container has no display. To look at the layout, use GTK's Broadway backend and open
`http://localhost:8080` in a browser (forward the port):

```bash
gtk4-broadwayd --port 8080 :5 &
GDK_BACKEND=broadway BROADWAY_DISPLAY=:5 cargo run --features demo
```

Broadway has no GPU and no sound: judge the player on a real desktop.

## Try it on a GNOME desktop

**Flatpak from CI (recommended).** Every CI run builds a development Flatpak,
`dev.baxyz.Flaneur.Devel`, on the GNOME 51 runtime, and uploads it as the `flaneur-x86_64`
artifact. Take it from the latest run of the
[Flatpak workflow](https://github.com/flaneur-app/app-gnome/actions/workflows/flatpak.yml)
(Artifacts, at the bottom of the run's page), or with `gh`:

```bash
run=$(gh run list -R flaneur-app/app-gnome -w Flatpak -L 1 --json databaseId -q '.[0].databaseId')
gh run download "$run" -R flaneur-app/app-gnome -n flaneur-x86_64   # writes flaneur.flatpak
flatpak install --user flaneur.flatpak    # pulls the GNOME runtime from Flathub if needed
flatpak run dev.baxyz.Flaneur.Devel
```

Installing a newer bundle replaces the previous one (`flatpak install --user --reinstall
flaneur.flatpak` if Flatpak complains). The local database and the followed feeds stay in
`~/.var/app/dev.baxyz.Flaneur.Devel/`.

The development build installs next to a future stable `dev.baxyz.Flaneur` and has a striped
header bar.

**Build the Flatpak on the desktop.** `flatpak-builder` does not work inside the dev container
(it cannot create the namespaces bubblewrap needs), but works on the desktop:

```bash
flatpak-builder --user --install-deps-from=flathub --force-clean --install \
  build-dir build-aux/dev.baxyz.Flaneur.Devel.yml
```

**Run from the dev container on the desktop's screen** with
[waypipe](https://gitlab.freedesktop.org/mstoeckl/waypipe), installed on both machines: it
forwards a Wayland window over SSH.

```bash
waypipe ssh you@dev-container-host 'cd /workspaces/app-gnome && cargo run'
```

## Hardware decoding

Playback uses GStreamer's `playbin3`, which picks the highest-ranked decoder for each stream. The
VA-API decoders of GStreamer's `va` plugin (`vah264dec`, `vavp9dec`, `vaav1dec`...) outrank the
software ones when the GPU can decode the codec. In the Flatpak, the GNOME runtime ships the
plugin and the Mesa VA drivers come with the GL extension; Intel GPUs need the
`org.freedesktop.Platform.VAAPI.Intel//26.08` extension (the GNOME 51 runtime is based on the
26.08 freedesktop runtime). `RUST_LOG=flaneur=info` logs the codecs that have a hardware decoder
when the first video plays.

## Development

See [AGENTS.md](AGENTS.md) for the structure, the conventions and the commands. The shared dev
container lives in [`flaneur-app/.dev`](https://github.com/flaneur-app/.dev).

## License

[AGPL-3.0-or-later](LICENSE).

The YouTube connector uses [RustyPipe](https://codeberg.org/ThetaDev/rustypipe), which is
GPL-3.0-only. The binary combines AGPL-3.0-or-later code with GPL-3.0-only code, which section 13
of both licenses allows; it is distributed under the terms of both, and its source is this
repository and the repositories it depends on.
