# Writes the corpus test image to stdout: a 160x100 PNG with a few flat
# shapes, generated here so the corpus holds no image of unknown origin.
import struct
import sys
import zlib

W, H = 160, 100


def pixel(x, y):
    if 10 <= x < 70 and 15 <= y < 85:
        return (31, 119, 180)
    if (x - 115) ** 2 + (y - 50) ** 2 <= 30 ** 2:
        return (255, 127, 14)
    if y >= 90:
        return (44, 160, 44)
    return (255, 255, 255)


def chunk(kind, data):
    body = kind + data
    return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body) & 0xFFFFFFFF)


raw = b"".join(b"\x00" + b"".join(bytes(pixel(x, y)) for x in range(W)) for y in range(H))
png = (
    b"\x89PNG\r\n\x1a\n"
    + chunk(b"IHDR", struct.pack(">IIBBBBB", W, H, 8, 2, 0, 0, 0))
    + chunk(b"IDAT", zlib.compress(raw, 9))
    + chunk(b"IEND", b"")
)
sys.stdout.buffer.write(png)
