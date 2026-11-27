#!/bin/sh
# Build the iPad app: target/package/alcyo-<version>-ipados.ipa, holding Alcyo.app with the voice detector and the
# meeting search model. The speech models are voice packs the app downloads, Apple's recognizer listening meanwhile
# (see packaging/models.sh).
#
# With an Apple Distribution certificate (IOS_CERT_P12, base64, and IOS_CERT_PASSWORD) and an App Store
# provisioning profile for app.alcyo.board (IOS_PROFILE, base64), it is signed for the App Store, and with
# UPLOAD=1 and the App Store Connect API key (APPLE_API_KEY_P8, base64, APPLE_API_KEY_ID,
# APPLE_API_ISSUER_ID) sent to TestFlight. Without them it is only built, which shows that it does.
#
# A local board model runs inside the app, as iOS lets an app start no other program: llama.cpp's server is linked
# into the program (crates/llama), Metal shaders and all, so nothing for it is copied here. It wants more memory than
# iPadOS gives an app by default, which the com.apple.developer.kernel.increased-memory-limit entitlement asks for;
# the app is signed with what its profile grants, so the profile must carry it. Needs a Mac with Xcode and the
# aarch64-apple-ios Rust target.
set -eu

root=$(cd "$(dirname "$0")/../.." && pwd)
cd "$root"
. packaging/models.sh

target=aarch64-apple-ios
min_os=17.0
export IPHONEOS_DEPLOYMENT_TARGET=$min_os

cargo build --release -p alcyo-tools
for what in glyphs voice-bundled search; do
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
mkdir -p "$app/Frameworks" "$app/assets/licenses"

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
# The launch screen's colour (UILaunchScreen in Info.plist): the board's paper, light and dark, as the app's first
# frame paints it (PAPER in crates/board/src/compose.rs, `paper` of the dark palette in crates/ui/src/theme.rs), so
# the system's screen and the app's are one surface.
mkdir -p "$icons/Assets.xcassets/LaunchPaper.colorset"
cat > "$icons/Assets.xcassets/LaunchPaper.colorset/Contents.json" <<'JSON'
{
  "colors": [
    { "idiom": "universal", "color": { "color-space": "srgb", "components": { "red": "0xFB", "green": "0xFB", "blue": "0xF9", "alpha": "1.000" } } },
    { "idiom": "universal", "appearances": [ { "appearance": "luminosity", "value": "dark" } ], "color": { "color-space": "srgb", "components": { "red": "0x07", "green": "0x07", "blue": "0x07", "alpha": "1.000" } } }
  ],
  "info": { "author": "xcode", "version": 1 }
}
JSON
xcrun actool "$icons/Assets.xcassets" --compile "$app" --platform iphoneos --minimum-deployment-target "$min_os" \
  --app-icon AppIcon --target-device ipad --output-partial-info-plist "$icons/partial.plist" > /dev/null
/usr/libexec/PlistBuddy -c "Merge $icons/partial.plist" "$app/Info.plist"
rm -rf "$icons"

# What Xcode writes into every app it builds, and App Store Connect will not take one without: which SDK,
# which Xcode, on which system.
sdk_version=$(xcrun --sdk iphoneos --show-sdk-version)
sdk_build=$(xcrun --sdk iphoneos --show-sdk-build-version)
xcode_version=$(xcodebuild -version | awk '/^Xcode/ { print $2 }')
xcode_build=$(xcodebuild -version | awk '/^Build version/ { print $3 }')
# 26.6 is written 2660, 26 is 2600.
xcode_number=$(printf '%s' "$xcode_version" | awk -F. '{ printf "%d%d%d", $1, $2, $3 }')
plist() { /usr/libexec/PlistBuddy -c "Add :$1 string $2" "$app/Info.plist"; }
plist DTPlatformName iphoneos
plist DTPlatformVersion "$sdk_version"
plist DTPlatformBuild "$sdk_build"
plist DTSDKName "iphoneos$sdk_version"
plist DTSDKBuild "$sdk_build"
plist DTXcode "$xcode_number"
plist DTXcodeBuild "$xcode_build"
plist DTCompiler com.apple.compilers.llvm.clang.1_0
plist BuildMachineOSBuild "$(sw_vers -buildVersion)"
echo "built with Xcode $xcode_version ($xcode_build), iOS SDK $sdk_version ($sdk_build)"

# Not "resources": the file system takes it for "Resources", which makes the app an old-style bundle to
# codesign, with every file beside it taken for code.
lib="$app/assets"
target/release/alcyo-tools pack-glyphs assets/hanzi "$lib/glyphs.pack"
copy_voice "$lib/voice"
copy_search "$lib/search"
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
  /usr/libexec/PlistBuddy -x -c 'Print :Entitlements' "$work/profile.plist" > "$work/granted.plist"
  # Signed with what the app uses, not all the profile grants: the App ID may carry more (Background GPU Access,
  # Background Inference), which the app does not use, as its model stops in the background (crates/agent).
  python3 -I - "$work/granted.plist" "$work/entitlements.plist" <<'PY'
import plistlib, sys
used = ["application-identifier", "com.apple.developer.team-identifier", "keychain-access-groups", "get-task-allow",
        "beta-reports-active", "com.apple.developer.kernel.increased-memory-limit"]
with open(sys.argv[1], "rb") as f:
    granted = plistlib.load(f)
for key in sorted(k for k in granted if k not in used):
    print(f"granted but not used: {key}")
with open(sys.argv[2], "wb") as f:
    plistlib.dump({k: granted[k] for k in used if k in granted}, f)
PY
  # Granted once Increased Memory Limit is turned on for app.alcyo.board in the Apple Developer portal and the profile
  # made again; without it the app is still signed, and the local model has iPadOS's default limit.
  if [ "$(/usr/libexec/PlistBuddy -c 'Print :com.apple.developer.kernel.increased-memory-limit' "$work/entitlements.plist" 2>/dev/null)" = true ]; then
    echo "the profile grants the increased memory limit"
  else
    echo "::warning::the profile does not grant com.apple.developer.kernel.increased-memory-limit: turn on Increased Memory Limit for app.alcyo.board in the Apple Developer portal and make the profile again"
  fi
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
