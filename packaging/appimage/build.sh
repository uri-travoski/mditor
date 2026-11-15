#!/bin/sh
# Build an AppImage from target/release. Run from the repository root.
set -eu
ID=dev.mditor.Mditor
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
mkdir -p build-appimage && cd build-appimage
for f in linuxdeploy-x86_64.AppImage; do
  [ -f "$f" ] || curl -fsSL -o "$f" "https://github.com/linuxdeploy/linuxdeploy/releases/download/1-alpha-20250213-2/$f"
done
[ -f linuxdeploy-plugin-gtk.sh ] || curl -fsSL -o linuxdeploy-plugin-gtk.sh https://raw.githubusercontent.com/linuxdeploy/linuxdeploy-plugin-gtk/master/linuxdeploy-plugin-gtk.sh
chmod +x linuxdeploy-x86_64.AppImage linuxdeploy-plugin-gtk.sh
rm -rf AppDir
export APPIMAGE_EXTRACT_AND_RUN=1 DEPLOY_GTK_VERSION=4 LINUXDEPLOY_OUTPUT_VERSION="$VERSION" PATH="$PWD:$PATH"

mkdir -p AppDir/usr/share
if [ -d /usr/share/gtksourceview-5 ]; then
  cp -r /usr/share/gtksourceview-5 AppDir/usr/share/
fi

./linuxdeploy-x86_64.AppImage --appdir AppDir \
  --executable ../target/release/mditor \
  --desktop-file ../data/$ID.desktop \
  --icon-file ../data/icons/$ID-256.png --icon-filename $ID \
  --plugin gtk --output appimage
mv mditor-*-x86_64.AppImage ../mditor-x86_64.AppImage
