#!/bin/sh
# Build the Mac app: target/package/alcyo-<version>-<arch>-macos.zip, holding Alcyo.app. It carries no
# speech models: it recognizes with Apple's own SpeechAnalyzer (SFSpeechRecognizer before macOS 26) until the
# voice packs chosen on first run are downloaded (see packaging/models.sh).
# With a Developer ID certificate they are signed with it, put in a disk image as well, and notarized with
# NOTARIZE=1 (packaging/macos/sign.sh); without one they are signed ad hoc, which Apple silicon requires to
# run at all, and a downloaded copy has to be opened with Control-click the first time.
# Needs Rust, Xcode, and network access the first time for models and llama-server.
set -eu

root=$(cd "$(dirname "$0")/../.." && pwd)
cd "$root"
. packaging/models.sh
. packaging/macos/sign.sh
setup_signing
rm -rf "$notary_list" target/package/notarized

cargo build --release -p alcyo-tools
for what in glyphs "llama" voice-bundled search pdfium; do
  target/release/alcyo-tools fetch $what
done
cargo build --release -p alcyo-ui

version=$(target/release/alcyo --version | cut -d' ' -f2)
mkdir -p target/package
glyphs=target/package/glyphs.pack
target/release/alcyo-tools pack-glyphs assets/hanzi "$glyphs"
speech=target/package/alcyo-speech
sh scripts/build-apple-speech.sh "$speech"

name="alcyo-$version-$(uname -m)-macos"
out="target/package/$name"
rm -rf "$out"
app="$out/Alcyo.app/Contents"
lib="$app/Resources/alcyo"
mkdir -p "$app/MacOS" "$lib/pdfium" "$lib/licenses"

install -m755 target/release/alcyo "$app/MacOS/alcyo"
strip "$app/MacOS/alcyo"
install -m755 "$speech" "$app/MacOS/alcyo-speech"
sed "s/@VERSION@/$version/g" packaging/macos/Info.plist.in > "$app/Info.plist"
cp packaging/icons/icon.icns "$app/Resources/icon.icns"

cp "$glyphs" "$lib/glyphs.pack"
cp -R resources/llama "$lib/llama"
rm -f "$lib/llama/.version"
cp target/pdfium/libpdfium.dylib "$lib/pdfium/"
copy_voice "$lib/voice"
copy_search "$lib/search"
cp LICENSE assets/licenses/* crates/fonts/files/OFL-*.txt assets/hanzi/ARPHICPL.TXT "$lib/licenses/"
# The fonts built into the program (crates/fonts) go with their licenses.
for f in OFL-DMSans.txt OFL-JetBrainsMono.txt OFL-NotoSansCJK.txt; do test -f "$lib/licenses/$f" || { echo "missing font license $f" >&2; exit 1; }; done
cp target/pdfium/LICENSE "$lib/licenses/pdfium-LICENSE"

sign_app "$out/Alcyo.app"

"$app/MacOS/alcyo" --self-check
ditto -c -k --keepParent "$out/Alcyo.app" "target/package/$name.zip"
if signed; then
  make_dmg "$out/Alcyo.app" "target/package/$name.dmg"
  ls -lh "target/package/$name.dmg"
fi
rm -rf "$out"
ls -lh "target/package/$name.zip"
# With NOTARIZE=1 every disk image is with Apple by now. Wait for them all and staple their tickets, unless
# NOTARY_WAIT=0 leaves that to a later step (notarize-wait.sh), as the release does.
if [ "${NOTARY_WAIT:-1}" = 1 ]; then
  notary_wait
fi
