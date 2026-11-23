#!/bin/sh
# Build the iPad app: target/package/alcyo-<version>-ipados.ipa, holding Alcyo.app with every speech model
# (variant "all", see packaging/models.sh) and the meeting search model.
#
# With an Apple Distribution certificate (IOS_CERT_P12, base64, and IOS_CERT_PASSWORD) and an App Store
# provisioning profile for app.alcyo.board (IOS_PROFILE, base64), it is signed for the App Store, and with
# UPLOAD=1 and the App Store Connect API key (APPLE_API_KEY_P8, base64, APPLE_API_KEY_ID,
# APPLE_API_ISSUER_ID) sent to TestFlight. Without them it is only built, which shows that it does.
#
# The board model runs remotely on the iPad, as iOS lets an app start no other program (see
# crates/ui/src/platform.rs). Needs a Mac with Xcode and the aarch64-apple-ios Rust target.
set -eu

root=$(cd "$(dirname "$0")/../.." && pwd)
cd "$root"
. packaging/models.sh

target=aarch64-apple-ios
min_os=17.0
export IPHONEOS_DEPLOYMENT_TARGET=$min_os

cargo build --release -p alcyo-tools
for what in glyphs voice search; do
  target/release/alcyo-tools fetch $what
done
cargo build --release --target $target -p alcyo-ui

version=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -1)
# Each upload needs a build number of its own, rising whatever sent it: the time, in UTC.
build=${BUILD_NUMBER:-$(date -u +%Y%m%d.%H%M)}
name="alcyo-$version-ipados"
out="target/package/$name"
app="$out/Payload/Alcyo.app"
rm -rf "$out"
mkdir -p "$app/Frameworks" "$app/assets/voice/speaker" "$app/assets/licenses"

install -m755 "target/$target/release/alcyo" "$app/alcyo"
strip -x "$app/alcyo" 2>/dev/null || true
sed -e "s/@VERSION@/$version/g" -e "s/@BUILD@/$build/g" -e "s/@MIN_OS@/$min_os/g" packaging/ios/Info.plist.in > "$app/Info.plist"

# sherpa-onnx comes as a framework, which the program loads from Frameworks.
framework=$(find target -path "*SherpaOnnxC.xcframework/ios-arm64/SherpaOnnxC.framework" -type d | head -1)
[ -n "$framework" ] || { echo "no SherpaOnnxC.framework for the device: did sherpa-onnx-sys download it?" >&2; exit 1; }
cp -R "$framework" "$app/Frameworks/"
install_name_tool -add_rpath @executable_path/Frameworks "$app/alcyo" 2>/dev/null || true

# The icon, compiled by actool from the 1024 px one.
icons=$(mktemp -d)
mkdir -p "$icons/Assets.xcassets/AppIcon.appiconset"
cp packaging/icons/ios-1024.png "$icons/Assets.xcassets/AppIcon.appiconset/icon.png"
cat > "$icons/Assets.xcassets/AppIcon.appiconset/Contents.json" <<'JSON'
{ "images": [ { "filename": "icon.png", "idiom": "universal", "platform": "ios", "size": "1024x1024" } ], "info": { "author": "xcode", "version": 1 } }
JSON
printf '{ "info": { "author": "xcode", "version": 1 } }\n' > "$icons/Assets.xcassets/Contents.json"
xcrun actool "$icons/Assets.xcassets" --compile "$app" --platform iphoneos --minimum-deployment-target "$min_os" \
  --app-icon AppIcon --target-device ipad --output-partial-info-plist "$icons/partial.plist" > /dev/null
/usr/libexec/PlistBuddy -c "Merge $icons/partial.plist" "$app/Info.plist"
rm -rf "$icons"

# Not "resources": the file system takes it for "Resources", which makes the app an old-style bundle to
# codesign, with every file beside it taken for code.
lib="$app/assets"
target/release/alcyo-tools pack-glyphs assets/hanzi "$lib/glyphs.pack"
cp resources/voice/silero_vad.onnx "$lib/voice/"
cp resources/voice/speaker/campplus_zh_en.onnx resources/voice/speaker/nemotron-3-diarization.int8.onnx "$lib/voice/speaker/"
copy_models all "$lib/voice"
copy_search "$lib/search"
printf 'all\n' > "$lib/variant"
cp LICENSE assets/licenses/* crates/fonts/files/OFL-*.txt assets/hanzi/ARPHICPL.TXT "$lib/licenses/"

# Only the program and the framework are code. codesign takes any other file marked executable, or
# carrying extended attributes, for code it must sign, and refuses the app.
xattr -cr "$app"
find "$app" -type f ! -path "$app/alcyo" ! -path "$app/Frameworks/*" -exec chmod 644 {} +
find "$app" -type d -exec chmod 755 {} +
ls -l "$app" | head -20

if [ -n "${IOS_CERT_P12:-}" ] && [ -n "${IOS_PROFILE:-}" ]; then
  work=$(mktemp -d)
  keychain="$work/alcyo-ios.keychain-db"
  password=$(openssl rand -hex 16)
  printf '%s' "$IOS_CERT_P12" | base64 --decode > "$work/cert.p12"
  security create-keychain -p "$password" "$keychain"
  security set-keychain-settings -lut 21600 "$keychain"
  security unlock-keychain -p "$password" "$keychain"
  security import "$work/cert.p12" -k "$keychain" -P "$IOS_CERT_PASSWORD" -T /usr/bin/codesign
  curl -fsSL -o "$work/AppleWWDRCAG3.cer" https://www.apple.com/certificateauthority/AppleWWDRCAG3.cer
  security import "$work/AppleWWDRCAG3.cer" -k "$keychain" || true
  security set-key-partition-list -S apple-tool:,apple: -s -k "$password" "$keychain" > /dev/null
  security list-keychains -d user -s "$keychain" $(security list-keychains -d user | tr -d '"')
  rm -f "$work/cert.p12"
  identity=$(security find-identity -v -p codesigning "$keychain" | awk '/Apple Distribution|iPhone Distribution/ { print $2; exit }')
  [ -n "$identity" ] || { echo "the certificate holds no Apple Distribution identity" >&2; exit 1; }

  printf '%s' "$IOS_PROFILE" | base64 --decode > "$app/embedded.mobileprovision"
  security cms -D -i "$app/embedded.mobileprovision" > "$work/profile.plist"
  /usr/libexec/PlistBuddy -x -c 'Print :Entitlements' "$work/profile.plist" > "$work/entitlements.plist"
  echo "signing with $(security find-identity -v -p codesigning "$keychain" | awk -F'"' '/Distribution/ { print $2; exit }'), profile $(/usr/libexec/PlistBuddy -c 'Print :Name' "$work/profile.plist")"
  codesign --force --sign "$identity" --timestamp=none "$app/Frameworks/SherpaOnnxC.framework"
  if ! codesign --force --sign "$identity" --timestamp=none --entitlements "$work/entitlements.plist" --verbose=4 "$app"; then
    # Why codesign took a resource for code: its type and attributes, and the bundle's top level.
    echo "--- signing the app failed; what is at its top level:" >&2
    for f in "$app"/*; do printf '%s: ' "$(basename "$f")"; file -b "$f"; xattr -l "$f" | sed 's/^/    /'; done
    ls -le@ "$app"
    exit 1
  fi
  codesign --verify --strict --deep --verbose=2 "$app"
  signed=1
else
  echo "no IOS_CERT_P12 or IOS_PROFILE: built, not signed" >&2
  signed=0
fi

(cd "$out" && zip -qry "../$name.ipa" Payload)
rm -rf "$out"
ls -lh "target/package/$name.ipa"

if [ "$signed" = 1 ] && [ "${UPLOAD:-0}" = 1 ]; then
  keys=$(mktemp -d)
  mkdir -p "$keys/private_keys"
  printf '%s' "$APPLE_API_KEY_P8" | base64 --decode > "$keys/private_keys/AuthKey_$APPLE_API_KEY_ID.p8"
  # altool looks for the key in ./private_keys.
  (cd "$keys" && xcrun altool --upload-app --type ios --file "$root/target/package/$name.ipa" \
    --apiKey "$APPLE_API_KEY_ID" --apiIssuer "$APPLE_API_ISSUER_ID") || { rm -rf "$keys"; exit 1; }
  rm -rf "$keys"
  echo "sent $name.ipa (build $build) to App Store Connect; it shows in TestFlight once Apple has processed it"
fi
