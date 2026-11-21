# Flâneur for GNOME

The GNOME app of [Flâneur](https://github.com/flaneur-app): a native GTK4/libadwaita client, packaged as a Flatpak. Screens, state and logic come from [`flaneur-app/core`](https://github.com/flaneur-app/core); this repository only holds the native GNOME interface.

> Flâneur is an independent, open-source project. It is not affiliated with, endorsed by or
> sponsored by Google, YouTube, Odysee, LBRY or any other service it can connect to. All
> trademarks belong to their owners.

**Status:** early development. The app shell (sidebar, adaptive layout, empty states, about
dialog), the video grid and the player work. No service connector is registered yet, so every
page shows "No services connected yet"; `flaneur --play <uri>` plays any video URI to try the
player.

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
`dev.baxyz.Flaneur.Devel`, on the GNOME 51 runtime. Download the `flaneur-x86_64` artifact of a
run from the [Actions tab](https://github.com/flaneur-app/app-gnome/actions) (or
`gh run download RUN_ID -R flaneur-app/app-gnome -n flaneur-x86_64`), then:

```bash
flatpak install --user ./flaneur.flatpak   # pulls the GNOME runtime from Flathub if needed
flatpak run dev.baxyz.Flaneur.Devel
flatpak run dev.baxyz.Flaneur.Devel --play https://download.blender.org/durian/trailer/sintel_trailer-480p.mp4
```

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
