#!/usr/bin/env bash
# Build chummer-rs and install it for the current user.
# The GUI is installed as `chummer-rs` so it never replaces another
# `chummer` launcher (for example one that runs Chummer5a under Wine).
#   ./install.sh            install to ~/.local
#   PREFIX=/usr/local sudo -E ./install.sh
set -euo pipefail
PREFIX="${PREFIX:-$HOME/.local}"
cd "$(dirname "$0")"

cargo build --release -p chummer-gui -p chummer-cli

install -Dm755 target/release/chummer-rs "$PREFIX/bin/chummer-rs"
install -Dm755 target/release/chummer-cli "$PREFIX/bin/chummer-cli"

share="$PREFIX/share/chummer-rs"
rm -rf "$share"
mkdir -p "$share"
cp -r resources/data resources/lang resources/sheets resources/customdata resources/export "$share/"
install -Dm644 resources/xml_license.txt "$share/xml_license.txt"

install -Dm644 packaging/chummer-rs.desktop "$PREFIX/share/applications/chummer-rs.desktop"
# Launchers may not have $PREFIX/bin on PATH.
sed -i "s|^Exec=chummer-rs |Exec=$PREFIX/bin/chummer-rs |" "$PREFIX/share/applications/chummer-rs.desktop"
install -Dm644 packaging/chummer-rs-mime.xml "$PREFIX/share/mime/packages/chummer-rs.xml"
install -Dm644 packaging/chummer-rs.svg "$PREFIX/share/icons/hicolor/scalable/apps/chummer-rs.svg"
command -v update-mime-database >/dev/null && update-mime-database "$PREFIX/share/mime" || true
command -v update-desktop-database >/dev/null && update-desktop-database "$PREFIX/share/applications" || true

echo "Installed chummer-rs and chummer-cli to $PREFIX/bin"
