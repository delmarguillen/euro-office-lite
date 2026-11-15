# mktxt.py OUT SAMPLE [z]: writes one of the text samples below, byte for byte.
# With "z", writes what saving the sample after typing Z at the end of its
# last line gives: the text with Z before its final line break.
import re
import sys


def utf8(text):
    return text.encode('utf-8')


def utf16le(text):
    return text.encode('utf-16-le')


def utf16be(text):
    return text.encode('utf-16-be')


def utf16le_bom(text):
    return b'\xff\xfe' + utf16le(text)


# More than 64 KiB in UTF-8, every character inside the BMP.
def long_bmp_text():
    line = 'Línea con acentos áéíóú ñ, euro € y cirílico Привет мир, número '
    text = ''
    i = 0
    while len(text) < 60000:
        text += line + str(i) + '\n'
        i += 1
    return text


BMP = 'Acentos áéíóú ñ\nCirílico Привет\nChino 中 y U+FFFD �\n'
NO_BOM = 'Accents áéíóú ñ\r\nCyrillic Привет мир\r\n'

SAMPLES = {
    # A character above U+FFFF in the middle line.
    'emoji-utf8': (utf8, 'x\na \U0001F600 b\ny\n'),
    'emoji-utf16le-bom': (utf16le_bom, 'x\na \U0001F600 b\ny\n'),
    # The only character above U+FFFF is in the last line.
    'long-emoji-end': (utf8, long_bmp_text() + 'última línea \U0001F600\n'),
    # Plain ASCII and one NUL, no final line break.
    'nul-after-text': (utf8, 'hello world\x00'),
    'utf16le-no-bom': (utf16le, NO_BOM),
    'utf16be-no-bom': (utf16be, NO_BOM),
    'bmp-utf8': (utf8, BMP),
    'html': (utf8, '<html><body><b>x</b></body></html>\n'),
    'csv-emoji': (utf8, 'name,icon\nsmile,\U0001F600\n\U00020000,2\n'),
}


def with_z(text):
    return re.sub(r'(\r?\n)\Z', r'Z\1', text)


out, sample = sys.argv[1], sys.argv[2]
encode, text = SAMPLES[sample]
if sys.argv[3:] == ['z']:
    text = with_z(text)
with open(out, 'wb') as f:
    f.write(encode(text))
