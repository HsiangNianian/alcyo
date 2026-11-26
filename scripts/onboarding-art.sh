#!/bin/sh
# The pictures beside the iPad's first run (crates/ui/art/onboarding-*.jpg), 1040 x 1668 px, twice the 520 x 834 pt the
# first run lays them out at:
#
#   1  the app icon's kingfisher (packaging/icons) without its tile, in a pool of light on the dark halftone
#   2  a board on the dark halftone, its ink in the dark palette
#   3  the same board on the board's dotted paper, light and dark, with the Pencil's contact dot where a circle ends
#
# The boards are drawn by the app's own layout and pen (`alcyo-tools onboarding-art`, crates/tools/src/onboarding_art.rs),
# so every mark is the one the app draws.
#
#   sh scripts/onboarding-art.sh
#
# Needs ImageMagick, rsvg-convert and Python 3.
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
art="$root/crates/ui/art"
W=1040
H=1668

python3 "$root/scripts/onboarding-art.py" ground "$work/ground.gray"
magick -size ${W}x$H -depth 8 "gray:$work/ground.gray" -colorspace sRGB -type TrueColor "PNG32:$work/ground.png"

# 1: the kingfisher, 600 px, its middle at the first run's focus, in light that fades into the dots.
magick "$root/packaging/icons/source/alcyo-1024.png" -depth 8 "rgba:$work/icon.rgba"
python3 "$root/scripts/onboarding-art.py" bird "$work/icon.rgba" "$work/bird.rgba"
magick -size 1024x1024 -depth 8 "rgba:$work/bird.rgba" -filter Lanczos -resize 600x600 "PNG32:$work/bird.png"
magick -size 960x960 radial-gradient:"rgba(255,253,248,1)-rgba(255,253,248,0)" -channel A -evaluate pow 0.7 +channel "PNG32:$work/light.png"
magick "PNG32:$work/ground.png" "$work/light.png" -geometry +40+240 -composite "$work/bird.png" -geometry +220+420 -composite \
  -alpha off -type TrueColor -quality 90 "$art/onboarding-1.jpg"

# 2 and 3: the board, 960 px wide, its middle at a share `y` of the picture's height (the focus in onboarding.rs).
cargo run -q -p alcyo-tools -- onboarding-art "$root" "$work" > /dev/null
place() { # <svg> <ground.png> <y share> <out.jpg>
  rsvg-convert -w 960 "$1" -o "$work/board.png"
  bh=$(magick identify -format %h "$work/board.png")
  top=$(awk -v h="$H" -v y="$3" -v bh="$bh" 'BEGIN { printf "%d", h * y - bh / 2 }')
  # A ground of greys alone would turn the coloured ink grey: it is made a colour picture first.
  magick "$2" -colorspace sRGB -type TrueColorAlpha "PNG32:$work/under.png"
  magick "PNG32:$work/under.png" "$work/board.png" -geometry +40+"$top" -composite -alpha off -type TrueColor -quality 90 "$4"
}
# Paper with the board's dots: 2 pt every 24 pt, at two pixels a point.
paper() { # <paper> <dot> <out.png>
  magick -size 48x48 "xc:$1" -fill "$2" -draw "circle 24,24 26,24" -write mpr:dot +delete -size ${W}x$H tile:mpr:dot "$3"
}
paper '#fbfbf9' '#e2e2e2' "$work/paper-light.png"
paper '#070707' '#262626' "$work/paper-dark.png"
place "$work/board-dark.svg" "$work/ground.png" 0.37 "$art/onboarding-2.jpg"
place "$work/board-light.svg" "$work/paper-light.png" 0.45 "$art/onboarding-3.jpg"
place "$work/board-dark.svg" "$work/paper-dark.png" 0.45 "$art/onboarding-3-dark.jpg"
echo "onboarding pictures written"
