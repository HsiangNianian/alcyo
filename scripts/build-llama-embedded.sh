#!/bin/sh
# Build llama.cpp's server as static libraries for the app to run inside itself (crates/llama), from our fork's tag:
#
#   sh scripts/build-llama-embedded.sh ios-arm64|macos-arm64
#
# Writes target/llama-embedded/llama-server-embedded-<platform>.tar.gz, holding lib/*.a and LICENSE. These go on the
# fork's release of the same tag, with their SHA-256 in .cargo/llama-embedded.sha256 (see crates/llama/build.rs);
# .github/workflows/llama-embedded.yml runs this on a Mac. Needs Xcode and CMake.
#
# A new llama.cpp: merge it into the fork's alcyo/<build> branch (never its master, whose push runs upstream's CI),
# tag it alcyo-<build>-<n>, set TAG and COMMIT here, and build, upload and record the archives anew.
set -eu

TAG=alcyo-b11954-1
COMMIT=264d066d6391a4d4848d91fc2d9d5997577b299b
platform=${1:?usage: build-llama-embedded.sh ios-arm64|macos-arm64}

root=$(cd "$(dirname "$0")/.." && pwd)
work="$root/target/llama-embedded"
src="$work/src-$TAG"
mkdir -p "$work"
if [ ! -d "$src" ]; then
  git clone --quiet --depth 1 --branch "$TAG" https://github.com/HsiangNianian/llama.cpp.git "$src"
fi
# The tag could be moved; the commit cannot.
[ "$(git -C "$src" rev-parse HEAD)" = "$COMMIT" ] || { echo "$TAG is not $COMMIT" >&2; exit 1; }

case "$platform" in
  ios-arm64)
    # The app's own deployment target (packaging/ios/package.sh).
    # Executables, which are not built, would otherwise be app bundles that need a place to be installed.
    system="-DCMAKE_SYSTEM_NAME=iOS -DCMAKE_SYSTEM_PROCESSOR=arm64 -DCMAKE_OSX_SYSROOT=iphoneos -DCMAKE_OSX_DEPLOYMENT_TARGET=17.0 -DCMAKE_MACOSX_BUNDLE=OFF"
    ;;
  macos-arm64)
    system="-DCMAKE_OSX_DEPLOYMENT_TARGET=11.0"
    ;;
  *) echo "no such platform: $platform" >&2; exit 1 ;;
esac

build="$work/build-$platform"
rm -rf "$build"
# The server only: no tests, examples or other tools, no web page (it serves none to the app), no HTTPS or
# downloads. Metal with its shaders built in, so there is no .metallib to ship beside the program.
# shellcheck disable=SC2086
cmake -S "$src" -B "$build" -G "Unix Makefiles" $system \
  -DCMAKE_BUILD_TYPE=Release \
  -DCMAKE_OSX_ARCHITECTURES=arm64 \
  -DBUILD_SHARED_LIBS=OFF \
  -DGGML_METAL=ON -DGGML_METAL_EMBED_LIBRARY=ON -DGGML_BLAS=ON -DGGML_NATIVE=OFF -DGGML_OPENMP=OFF \
  -DLLAMA_BUILD_TESTS=OFF -DLLAMA_BUILD_EXAMPLES=OFF -DLLAMA_BUILD_TOOLS=ON -DLLAMA_BUILD_SERVER=ON -DLLAMA_BUILD_APP=OFF \
  -DLLAMA_BUILD_UI=OFF -DLLAMA_USE_PREBUILT_UI=OFF -DLLAMA_OPENSSL=OFF -DLLAMA_CONNECT=OFF
cmake --build "$build" --config Release --target llama-server-impl -j "$(sysctl -n hw.ncpu)"

out="$work/out-$platform"
rm -rf "$out"
mkdir -p "$out/lib"
find "$build" -name '*.a' -exec cp {} "$out/lib/" \;
cp "$src/LICENSE" "$out/LICENSE"
printf '%s %s %s\n' "$TAG" "$COMMIT" "$platform" > "$out/VERSION"
ls -l "$out/lib"
archive="$work/llama-server-embedded-$platform.tar.gz"
tar -czf "$archive" -C "$out" lib LICENSE VERSION
shasum -a 256 "$archive" | sed "s|  .*/|  |"
