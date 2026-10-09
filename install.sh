#!/usr/bin/env bash
# Build chummer-rs and install it for the current user.
# The GUI is installed as `chummer-rs` so it never replaces another
# `chummer` launcher (for example one that runs Chummer5a under Wine).
#   ./install.sh            install to ~/.local
#   PREFIX=/usr/local sudo -E ./install.sh
set -euo pipefail
PREFIX="${PREFIX:-$HOME/.local}"
cd "$(dirname "$0")"

cargo build --release -p chummer-gui -p chummer-cli -p chummer-authority

install -Dm755 target/release/chummer-rs "$PREFIX/bin/chummer-rs"
install -Dm755 target/release/chummer-cli "$PREFIX/bin/chummer-cli"
install -Dm755 target/release/chummer-authority "$PREFIX/bin/chummer-authority"

# The program's resources go in share/chummer-rs/resources. In ~/.local,
# share/chummer-rs is also the user's own data folder (custom data,
# sheets, kits, backups...), so only the resources folder is replaced.
share="$PREFIX/share/chummer-rs"
res="$share/resources"
if [ -d "$share/data" ]; then
    # An older install.sh put the resources directly in $share. Remove
    # what it installed; files the user added to sheets/ or customdata/
    # stay.
    rm -rf "$share/data" "$share/lang" "$share/export" "$share/xml_license.txt"
    for d in sheets customdata; do
        [ -d "$share/$d" ] || continue
        for f in resources/"$d"/*; do
            rm -rf "$share/$d/$(basename "$f")"
        done
    done
fi
rm -rf "$res"
mkdir -p "$res"
cp -r resources/data resources/lang resources/sheets resources/customdata resources/export "$res/"
install -Dm644 resources/xml_license.txt "$res/xml_license.txt"

install -Dm644 packaging/chummer-rs.desktop "$PREFIX/share/applications/chummer-rs.desktop"
# Launchers may not have $PREFIX/bin on PATH.
sed -i "s|^Exec=chummer-rs |Exec=$PREFIX/bin/chummer-rs |" "$PREFIX/share/applications/chummer-rs.desktop"
install -Dm644 packaging/chummer-rs-mime.xml "$PREFIX/share/mime/packages/chummer-rs.xml"
install -Dm644 packaging/chummer-rs.svg "$PREFIX/share/icons/hicolor/scalable/apps/chummer-rs.svg"
command -v update-mime-database >/dev/null && update-mime-database "$PREFIX/share/mime" || true
command -v update-desktop-database >/dev/null && update-desktop-database "$PREFIX/share/applications" || true
# Open invite links (chummer-rs://join/...) with chummer-rs.
command -v xdg-mime >/dev/null && xdg-mime default chummer-rs.desktop x-scheme-handler/chummer-rs || true

echo "Installed chummer-rs, chummer-cli and chummer-authority to $PREFIX/bin"
