"""Generate the original R-Music vector-shaped Windows icon without external libraries."""
import pathlib
import struct

size = 64
pixels = bytearray()
for y in reversed(range(size)):
    for x in range(size):
        note = (19 <= x < 23 and 19 <= y < 44) or (41 <= x < 45 and 13 <= y < 38)
        note |= 19 <= x < 45 and 13 <= y - (45 - x) * 6 / 26 < 18
        note |= (x - 16) ** 2 + (y - 44) ** 2 <= 49 or (x - 38) ** 2 + (y - 38) ** 2 <= 49
        r, g, b = (168, 223, 200) if note else (17, 19, 21)
        pixels += bytes((b, g, r, 255))
mask = bytes(((size + 31) // 32) * 4 * size)
bitmap = struct.pack("<IIIHHIIIIII", 40, size, size * 2, 1, 32, 0, len(pixels), 0, 0, 0, 0) + pixels + mask
ico = struct.pack("<HHH", 0, 1, 1) + struct.pack("<BBBBHHII", size, size, 0, 0, 1, 32, len(bitmap), 22) + bitmap
path = pathlib.Path(__file__).parent / "src-tauri" / "icons" / "icon.ico"
path.write_bytes(ico)
