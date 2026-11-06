"""Mod Engine app icon: 32x32 pixel art drawn by hand, scaled with nearest neighbour.

    python3 assets/icon/icon.py      # writes icon-32.png, icon-256.png and icon-512.png here

No dependencies: the PNG is written with zlib. Edit SLIME / PAL and run it again.
"""
import os
import struct
import zlib

N = 32
PAL = {
    'K': (20, 22, 48),     # outline
    'S': (88, 140, 255),   # sky, top band
    's': (63, 116, 250),   # sky
    'b': (44, 92, 232),    # sky, bottom band
    'D': (33, 72, 200),    # tile shade, bottom edge
    'H': (130, 172, 255),  # tile highlight, top edge
    'G': (104, 214, 110),  # slime
    'g': (164, 240, 150),  # slime light
    'd': (46, 150, 82),    # slime shade
    'k': (20, 22, 48),     # eyes / mouth
    'w': (255, 255, 255),  # eye shine, star core
    'P': (246, 136, 160),  # cheeks
    'T': (214, 150, 84),   # dirt top
    'R': (150, 92, 52),    # dirt
    'r': (118, 70, 40),    # dirt dots
    'y': (255, 226, 120),  # star glow
}

SLIME = [
    ".......KKKKKK.......",
    ".....KKggGGGGKK.....",
    "....KgwwgGGGGGGK....",
    "...KgwwgGGGGGGGGK...",
    "..KGggGGGGGGGGGGGK..",
    "..KGGGGGGGGGGGGGGK..",
    ".KGGGGkkGGGGkkGGGGK.",
    ".KGGGGkwGGGGkwGGGGK.",
    ".KGGGGkkGGGGkkGGGGK.",
    ".KGGPGGGGkkGGGGPGGK.",
    "KGGGGGGGGGGGGGGGGGGK",
    "KdGGGGGGGGGGGGGGGGdK",
    "KddGGGGGGGGGGGGGGddK",
    ".KddddddddddddddddK.",
    "..KKKKKKKKKKKKKKKK..",
]


def tile():
    g = [['.'] * N for _ in range(N)]
    for y in range(N):
        for x in range(N):
            # Rounded corners: cut 2 pixels diagonally at each corner.
            cx, cy = min(x, N - 1 - x), min(y, N - 1 - y)
            if cx + cy < 2:
                continue
            edge = cx == 0 or cy == 0 or cx + cy == 2
            if edge:
                g[y][x] = 'K'
            elif y == 1 or (cy == 1 and y < N // 2) or (cx + cy == 3 and y < N // 2):
                g[y][x] = 'H'
            elif y >= N - 2:
                g[y][x] = 'D'
            else:
                g[y][x] = 'S' if y < 9 else 's' if y < 17 else 'b'
    # Ground: dirt with a light top and a few dots.
    for y in range(25, N - 2):
        for x in range(1, N - 1):
            if g[y][x] in 'Ssb':
                g[y][x] = 'T' if y == 25 else 'R'
    for x, y in [(4, 27), (9, 28), (15, 27), (21, 28), (26, 27), (12, 29), (24, 29)]:
        g[y][x] = 'r'
    # Two small stars.
    for sx, sy in [(6, 6), (25, 9)]:
        g[sy][sx] = 'w'
        for dx, dy in [(-1, 0), (1, 0), (0, -1), (0, 1)]:
            g[sy + dy][sx + dx] = 'y'
    g[4][19] = 'y'
    # The slime sits on the ground.
    ox, oy = 6, 11
    for j, row in enumerate(SLIME):
        for i, c in enumerate(row):
            if c != '.':
                g[oy + j][ox + i] = c
    return g


def png(grid, scale):
    w = len(grid[0]) * scale
    raw = bytearray()
    for row in grid:
        line = bytearray([0])
        for c in row:
            px = (0, 0, 0, 0) if c == '.' else PAL[c] + (255,)
            line += bytes(px) * scale
        raw += bytes(line) * scale

    def chunk(t, d):
        return struct.pack('>I', len(d)) + t + d + struct.pack('>I', zlib.crc32(t + d) & 0xffffffff)

    return (b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', w, w, 8, 6, 0, 0, 0))
            + chunk(b'IDAT', zlib.compress(bytes(raw), 9)) + chunk(b'IEND', b''))


if __name__ == '__main__':
    here = os.path.dirname(os.path.abspath(__file__))
    g = tile()
    for s in (1, 8, 16):
        with open(os.path.join(here, f'icon-{N * s}.png'), 'wb') as f:
            f.write(png(g, s))
    print('\n'.join(''.join(r) for r in g))
