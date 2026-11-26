"""Pieces of the iPad's first-run pictures, for scripts/onboarding-art.sh.

  onboarding-art.py ground <out.gray>             the dark halftone: dots every 12 px, brighter towards the middle
  onboarding-art.py bird <in.rgba> <out.rgba>     the app icon's kingfisher with its white tile taken out as alpha

Raw 8-bit pixels in and out (ImageMagick's gray: and rgba:), 1040 x 1668 for the ground, 1024 x 1024 for the bird.
"""
import math
import sys

W, H = 1040, 1668


def ground(out):
    """Dots on near black, each as bright as a soft oval of light centred on the picture makes it."""
    background, peak, low = 11, 100, 13
    pixels = bytearray([background]) * (W * H)
    for cy in range(5, H, 12):
        for cx in range(7, W, 12):
            dx, dy = (cx - W / 2) / 310, (cy - H / 2) / 520
            value = low + (peak - low) * math.exp(-(dx * dx + dy * dy))
            for y in range(cy - 4, cy + 5):
                for x in range(cx - 4, cx + 5):
                    if 0 <= x < W and 0 <= y < H:
                        a = max(0.0, min(1.0, 3.4 - math.hypot(x - cx, y - cy)))
                        if a > 0:
                            pixels[y * W + x] = max(pixels[y * W + x], int(background + (value - background) * a))
    open(out, 'wb').write(bytes(pixels))


def bird(src, out):
    """White taken out as alpha ("colour to alpha"): each colour is what, laid over white, gives the pixel back, so
    the artwork shows as it was over white and keeps its colours over anything else."""
    d = open(src, 'rb').read()
    res = bytearray(len(d))
    for i in range(0, len(d), 4):
        r, g, b, a0 = d[i], d[i + 1], d[i + 2], d[i + 3]
        a = max(255 - r, 255 - g, 255 - b) / 255.0
        if a0 == 0 or a < 0.02:
            continue
        c = [min(255, max(0, round((v - 255 * (1 - a)) / a))) for v in (r, g, b)]
        res[i:i + 4] = bytes([c[0], c[1], c[2], round(a * a0)])
    open(out, 'wb').write(bytes(res))


if __name__ == '__main__':
    {'ground': lambda: ground(sys.argv[2]), 'bird': lambda: bird(sys.argv[2], sys.argv[3])}[sys.argv[1]]()
