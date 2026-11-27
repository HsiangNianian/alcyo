#!/bin/sh
# Build the Linux package: target/package/alcyo-<version>-<arch>-linux.tar.gz. It unpacks into a working
# app, and install.sh inside puts it on the system. Speech models are voice packs the app downloads (see
# packaging/models.sh).
# Needs Rust, and network access the first time for models and llama-server.
set -eu

root=$(cd "$(dirname "$0")/../.." && pwd)
cd "$root"
. packaging/models.sh

cargo build --release -p alcyo-tools
for what in glyphs "llama linux-x64" voice-bundled search pdfium; do
  target/release/alcyo-tools fetch $what
done
cargo build --release -p alcyo-ui

version=$(target/release/alcyo --version | cut -d' ' -f2)
glyphs=target/package/glyphs.pack
mkdir -p target/package
target/release/alcyo-tools pack-glyphs assets/hanzi "$glyphs"

name="alcyo-$version-$(uname -m)-linux"
out="target/package/$name"
rm -rf "$out"
lib="$out/lib/alcyo"
mkdir -p "$out/bin" "$lib/pdfium" "$out/share/licenses/alcyo"

install -m755 target/release/alcyo "$out/bin/alcyo"
strip "$out/bin/alcyo"

cp "$glyphs" "$lib/glyphs.pack"
cp -r resources/llama "$lib/llama"
rm -f "$lib/llama/.version"
cp target/pdfium/libpdfium.so "$lib/pdfium/"
copy_voice "$lib/voice"
copy_search "$lib/search"

cp LICENSE assets/licenses/* crates/fonts/files/OFL-*.txt assets/hanzi/ARPHICPL.TXT "$out/share/licenses/alcyo/"
# The fonts built into the program (crates/fonts) go with their licenses.
for f in OFL-DMSans.txt OFL-JetBrainsMono.txt OFL-NotoSansCJK.txt; do test -f "$out/share/licenses/alcyo/$f" || { echo "missing font license $f" >&2; exit 1; }; done
cp target/pdfium/LICENSE "$out/share/licenses/alcyo/pdfium-LICENSE"
install -m644 packaging/icons/128x128@2x.png "$out/share/app.alcyo.board.png"
install -m644 packaging/linux/app.alcyo.board.desktop.in "$out/share/app.alcyo.board.desktop.in"
install -m755 packaging/linux/install.sh "$out/install.sh"
printf '%s\n' "$version" > "$out/VERSION"

"$out/bin/alcyo" --self-check
tar -C target/package -czf "target/package/$name.tar.gz" "$name"
rm -rf "$out"
ls -lh "target/package/$name.tar.gz"
