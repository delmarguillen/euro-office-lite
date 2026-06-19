"""Generate font thumbnail sprite for ComboBoxFonts.

Each row is 300x28 pixels showing the font name rendered in that font.
The order matches __fonts_infos in AllFonts.js (the thumbnail index = row).
Generates sprites at 1x, 1.25x, 1.5x, 1.75x, and 2x DPI.
"""

import os
import sys
from PIL import Image, ImageDraw, ImageFont

SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
PROJECT_ROOT = os.path.dirname(SCRIPT_DIR)
FONTS_DIR = os.path.join(PROJECT_ROOT, "src", "fonts")
OUTPUT_DIR = os.path.join(PROJECT_ROOT, "src", "sdkjs", "common", "Images")

BASE_WIDTH = 300
BASE_HEIGHT = 28
FONT_SIZE_BASE = 16
TEXT_Y_OFFSET_BASE = 4
TEXT_X_OFFSET = 10

FONT_ENTRIES = [
    ("Liberation Sans",  "LiberationSans-Regular.ttf"),
    ("Liberation Serif", "LiberationSerif-Regular.ttf"),
    ("Liberation Mono",  "LiberationMono-Regular.ttf"),
    ("Carlito",          "Carlito-Regular.ttf"),
    ("Arial",            "LiberationSans-Regular.ttf"),
    ("Helvetica",        "LiberationSans-Regular.ttf"),
    ("Times New Roman",  "LiberationSerif-Regular.ttf"),
    ("Courier New",      "LiberationMono-Regular.ttf"),
    ("Calibri",          "Carlito-Regular.ttf"),
    ("Cambria",          "LiberationSerif-Regular.ttf"),
]

DPI_VARIANTS = [
    (1.0,  ""),
    (1.25, "@1.25x"),
    (1.5,  "@1.5x"),
    (1.75, "@1.75x"),
    (2.0,  "@2x"),
]


def generate_sprite(scale, suffix):
    w = int(BASE_WIDTH * scale)
    h = int(BASE_HEIGHT * scale)
    font_size = int(FONT_SIZE_BASE * scale)
    y_offset = int(TEXT_Y_OFFSET_BASE * scale)
    x_offset = int(TEXT_X_OFFSET * scale)

    rows = len(FONT_ENTRIES)
    sprite = Image.new("RGBA", (w, h * rows), (255, 255, 255, 0))
    draw = ImageDraw.Draw(sprite)

    for i, (name, ttf_file) in enumerate(FONT_ENTRIES):
        ttf_path = os.path.join(FONTS_DIR, ttf_file)
        try:
            font = ImageFont.truetype(ttf_path, font_size)
        except Exception:
            font = ImageFont.load_default()

        y = i * h + y_offset
        draw.text((x_offset, y), name, fill=(0, 0, 0, 255), font=font)

    out_path = os.path.join(OUTPUT_DIR, f"fonts_thumbnail{suffix}.png")
    sprite.save(out_path, "PNG")
    print(f"  Generated: {os.path.basename(out_path)} ({w}x{h * rows})")


def main():
    os.makedirs(OUTPUT_DIR, exist_ok=True)
    print("Generating font thumbnail sprites...")

    for scale, suffix in DPI_VARIANTS:
        generate_sprite(scale, suffix)

    print("Done.")


if __name__ == "__main__":
    main()
