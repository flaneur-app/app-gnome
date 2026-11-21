#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# Installs the release binary and the desktop integration files under a prefix:
#
#   build-aux/install.sh PREFIX [APP_ID]
#
# APP_ID (default dev.baxyz.Flaneur) names the desktop file, the AppStream metadata and the icons,
# and replaces the id inside them, so a development build (dev.baxyz.Flaneur.Devel) installs next
# to a stable one. Run it from the repository root after `cargo build --release`.
set -eu

prefix=${1:?usage: build-aux/install.sh PREFIX [APP_ID]}
app_id=${2:-dev.baxyz.Flaneur}
base=dev.baxyz.Flaneur

# Copies a text file, replacing the base id with the app id.
rewrite() {
  mkdir -p "$(dirname "$2")"
  sed "s/dev\.baxyz\.Flaneur/$app_id/g" "$1" > "$2"
  chmod 644 "$2"
}

install -Dm755 target/release/flaneur "$prefix/bin/flaneur"
rewrite "data/$base.desktop" "$prefix/share/applications/$app_id.desktop"
rewrite "data/$base.metainfo.xml" "$prefix/share/metainfo/$app_id.metainfo.xml"
install -Dm644 "data/icons/hicolor/scalable/apps/$base.svg" \
  "$prefix/share/icons/hicolor/scalable/apps/$app_id.svg"
install -Dm644 "data/icons/hicolor/symbolic/apps/$base-symbolic.svg" \
  "$prefix/share/icons/hicolor/symbolic/apps/$app_id-symbolic.svg"
