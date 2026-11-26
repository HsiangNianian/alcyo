#!/bin/sh
# Rebuild every app and website icon from the artwork in source/: the kingfisher on its white rounded tile,
# transparent outside the corners, at 32, 64, 192, 512 and 1024 px, and a Windows icon holding 16 to 256 px.
# The sizes the artwork comes in are used as they are; the others are scaled from the nearest larger one.
# Needs ImageMagick and png2icns (libicns). The outputs are committed, so packaging never runs this.
#   source/alcyo.ico              -> icon.ico               Windows, embedded in alcyo.exe by crates/ui/build.rs
#   source/alcyo.ico (256 px)     -> 128x128@2x.png         Linux hicolor 256x256
#   source/alcyo-1024.png         -> icon.icns              macOS bundle (Big Sur grid: 824 px tile, with shadow)
#   source/alcyo-1024.png         -> ios-1024.png           App Store Connect: full bleed, no alpha
#   source/alcyo-{32,64,192}.png  -> landing/public/favicon-*.png, apple-touch-icon.png   website
set -eu
cd "$(dirname "$0")"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
src=source

cp "$src/alcyo.ico" icon.ico
magick "$src/alcyo.ico[7]" -strip 128x128@2x.png

# macOS draws icons on a 1024 canvas with the tile at 824 px and a soft shadow under it; the artwork's
# tile fills its canvas, so it is scaled into that grid here. The shadow is a grey image, so the result is
# written as PNG32, or ImageMagick would keep it grey.
magick "$src/alcyo-1024.png" -resize 824x824 "$tmp/tile.png"
magick "$tmp/tile.png" -alpha extract -background black -alpha shape -channel A -evaluate multiply 0.3 +channel \
  -background none -gravity center -extent 1024x1024 -page +0+10 -flatten -blur 0x10 "$tmp/shadow.png"
magick "$tmp/shadow.png" -colorspace sRGB -type TrueColorAlpha "$tmp/tile.png" -gravity center -composite "PNG32:$tmp/mac-1024.png"
for s in 16 32 48 128 256 512; do magick "$tmp/mac-1024.png" -filter Lanczos -resize ${s}x$s "PNG32:$tmp/mac-$s.png"; done
png2icns icon.icns "$tmp"/mac-16.png "$tmp"/mac-32.png "$tmp"/mac-48.png "$tmp"/mac-128.png \
  "$tmp"/mac-256.png "$tmp"/mac-512.png "$tmp"/mac-1024.png

# iOS masks the corners itself and refuses alpha: the transparent corners become the tile's white.
magick "$src/alcyo-1024.png" -background white -flatten -strip -alpha off -define png:color-type=2 ios-1024.png

# The website (crates/site-gen links these from every page): tab icons at 32 and 64 px, a 192 px one for
# Android and other large uses, and the iOS home-screen icon, which iOS masks like the App Store one.
site=../../landing/public
cp "$src/alcyo-32.png" "$site/favicon-32.png"
cp "$src/alcyo-64.png" "$site/favicon.png"
cp "$src/alcyo-192.png" "$site/favicon-192.png"
magick "$src/alcyo-192.png" -background white -flatten -filter Lanczos -resize 180x180 -strip -alpha off "$site/apple-touch-icon.png"
