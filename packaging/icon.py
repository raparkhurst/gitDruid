#!/usr/bin/env python3
"""Draws gitDruid's icon, at every size the platforms ask for.

The icon is generated rather than checked in as a binary blob so that it can
be changed by editing numbers here: colours come from the Console palette in
`src/ui/theme.rs`, and the shape is the thing the app is for — a commit graph,
with one branch leaving the mainline and rejoining it.

Pure standard library: PNG is a container around zlib, which Python has, and
the drawing is coverage-sampled circles and line segments. No image library is
needed to build a release.
"""

import math
import os
import struct
import sys
import zlib

BACKGROUND = (0x1B, 0x1A, 0x18)
MAINLINE = (0x7F, 0xA7, 0xD9)
BRANCH = (0x86, 0xB8, 0x7A)
HEAD = (0xD9, 0x77, 0x57)

# Rendered once at this size and boxed down; small sizes stay legible because
# the shapes are heavy relative to the tile.
MASTER = 1024
SUPERSAMPLE = 2


def rounded_rect(x0, y0, x1, y1, radius):
    """Coverage test for a rounded rectangle."""

    def inside(x, y):
        if x < x0 or x > x1 or y < y0 or y > y1:
            return False

        cx = min(max(x, x0 + radius), x1 - radius)
        cy = min(max(y, y0 + radius), y1 - radius)

        return (x - cx) ** 2 + (y - cy) ** 2 <= radius * radius

    return inside, (x0, y0, x1, y1)


def disc(cx, cy, radius):
    def inside(x, y):
        return (x - cx) ** 2 + (y - cy) ** 2 <= radius * radius

    return inside, (cx - radius, cy - radius, cx + radius, cy + radius)


def polyline(points, width):
    """A run of segments with round joins, as one coverage test."""

    half = width / 2.0
    segments = list(zip(points, points[1:]))

    def inside(x, y):
        for (ax, ay), (bx, by) in segments:
            dx, dy = bx - ax, by - ay
            length = dx * dx + dy * dy

            if length == 0:
                t = 0.0
            else:
                t = max(0.0, min(1.0, ((x - ax) * dx + (y - ay) * dy) / length))

            px, py = ax + t * dx, ay + t * dy

            if (x - px) ** 2 + (y - py) ** 2 <= half * half:
                return True

        return False

    xs = [p[0] for p in points]
    ys = [p[1] for p in points]

    return inside, (min(xs) - half, min(ys) - half, max(xs) + half, max(ys) + half)


def curve(start, control_a, control_b, end, width, steps=48):
    """A cubic bezier, as a polyline dense enough not to show its corners."""

    points = []

    for step in range(steps + 1):
        t = step / steps
        u = 1 - t

        x = (
            u * u * u * start[0]
            + 3 * u * u * t * control_a[0]
            + 3 * u * t * t * control_b[0]
            + t * t * t * end[0]
        )
        y = (
            u * u * u * start[1]
            + 3 * u * u * t * control_a[1]
            + 3 * u * t * t * control_b[1]
            + t * t * t * end[1]
        )

        points.append((x, y))

    return polyline(points, width)


def draw(size):
    """Renders the icon at `size`, returning RGBA bytes."""

    scale = size / 1024.0
    sample = SUPERSAMPLE
    step = 1.0 / sample
    weight = 1.0 / (sample * sample)

    def u(value):
        return value * scale

    # The tile, then the graph on top of it: a mainline, a branch leaving it
    # and coming back, and the three commits that matter.
    lane_left = u(360)
    lane_right = u(664)
    stroke = u(54)

    shapes = [
        (rounded_rect(u(40), u(40), u(984), u(984), u(220)), BACKGROUND),
        (polyline([(lane_left, u(150)), (lane_left, u(874))], stroke), MAINLINE),
        (
            curve(
                (lane_left, u(700)),
                (lane_right, u(660)),
                (lane_right, u(600)),
                (lane_right, u(512)),
                stroke,
            ),
            BRANCH,
        ),
        (
            curve(
                (lane_right, u(512)),
                (lane_right, u(424)),
                (lane_right, u(364)),
                (lane_left, u(324)),
                stroke,
            ),
            BRANCH,
        ),
        (disc(lane_right, u(512), u(88)), BRANCH),
        (disc(lane_left, u(700), u(96)), MAINLINE),
        (disc(lane_left, u(324), u(112)), HEAD),
    ]

    pixels = bytearray(size * size * 4)

    for (inside, bounds), colour in shapes:
        x0 = max(0, int(math.floor(bounds[0])))
        y0 = max(0, int(math.floor(bounds[1])))
        x1 = min(size - 1, int(math.ceil(bounds[2])))
        y1 = min(size - 1, int(math.ceil(bounds[3])))

        red, green, blue = colour

        for y in range(y0, y1 + 1):
            row = y * size * 4

            for x in range(x0, x1 + 1):
                covered = 0

                for sy in range(sample):
                    py = y + (sy + 0.5) * step

                    for sx in range(sample):
                        if inside(x + (sx + 0.5) * step, py):
                            covered += 1

                if covered == 0:
                    continue

                alpha = covered * weight
                offset = row + x * 4

                # Ordinary source-over, so a shape softens into whatever is
                # already under it rather than cutting a hole in it.
                existing = pixels[offset + 3] / 255.0
                result = alpha + existing * (1 - alpha)

                for channel, value in enumerate((red, green, blue)):
                    under = pixels[offset + channel] / 255.0 * existing
                    mixed = (value / 255.0 * alpha + under * (1 - alpha)) / result

                    pixels[offset + channel] = int(round(mixed * 255))

                pixels[offset + 3] = int(round(result * 255))

    return bytes(pixels)


def downsample(pixels, size, target):
    """Box filter, which is all that is wanted going from a power of two."""

    factor = size // target
    out = bytearray(target * target * 4)

    for y in range(target):
        for x in range(target):
            totals = [0, 0, 0, 0]

            for sy in range(factor):
                row = ((y * factor) + sy) * size * 4

                for sx in range(factor):
                    offset = row + ((x * factor) + sx) * 4

                    for channel in range(4):
                        totals[channel] += pixels[offset + channel]

            offset = (y * target + x) * 4
            count = factor * factor

            for channel in range(4):
                out[offset + channel] = totals[channel] // count

    return bytes(out)


def write_png(path, size, pixels):
    raw = b"".join(
        b"\x00" + pixels[y * size * 4 : (y + 1) * size * 4] for y in range(size)
    )

    def chunk(tag, data):
        body = tag + data

        return (
            struct.pack(">I", len(data))
            + body
            + struct.pack(">I", zlib.crc32(body) & 0xFFFFFFFF)
        )

    header = struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0)

    with open(path, "wb") as handle:
        handle.write(
            b"\x89PNG\r\n\x1a\n"
            + chunk(b"IHDR", header)
            + chunk(b"IDAT", zlib.compress(raw, 9))
            + chunk(b"IEND", b"")
        )


def main():
    out = sys.argv[1] if len(sys.argv) > 1 else "packaging/icons"
    os.makedirs(out, exist_ok=True)

    print(f"rendering {MASTER}px master…", flush=True)
    master = draw(MASTER)
    write_png(os.path.join(out, f"{MASTER}.png"), MASTER, master)

    for size in (512, 256, 128, 64, 32, 16):
        print(f"  {size}px", flush=True)
        write_png(os.path.join(out, f"{size}.png"), size, downsample(master, MASTER, size))

    # macOS asks for the odd sizes in between as well.
    for size in (48, 1024):
        if size == 1024:
            continue

        print(f"  {size}px", flush=True)
        scaled = downsample(master, MASTER, 64)
        write_png(os.path.join(out, "48.png"), 48, draw(48))

    print(f"written to {out}/")


if __name__ == "__main__":
    main()
