# mkmd.py OUT SAMPLE [z]: writes one of the Markdown samples below, byte for
# byte. With "z", writes what saving the sample after typing Z at the end of
# the document gives: Ctrl+End in the Markdown editor goes past the final
# line break, so the Z comes after it. A RAW sample is written as it is and
# takes no "z".
import sys

BOM = b'\xef\xbb\xbf'
TEXT = '# Notes\n\nAccents áéíóú ñ, Cyrillic Привет.\n\n- one\n- two\n'
# An emoji and a CJK Extension B character, both above U+FFFF.
EMOJI = '# Emoji\n\nsmile \U0001F600 and \U00020000\n'
# CRLF, LF and a lone CR in one file: no single line end gives it back.
MIXED_EOL = '# Notes\r\n\r\nCRLF line\r\nLF line\nCR line\rlast\r\n'

SAMPLES = {
    'utf8': (b'', TEXT),
    'utf8-bom': (BOM, TEXT),
    'crlf': (b'', TEXT.replace('\n', '\r\n')),
    'emoji': (b'', EMOJI),
    'mixed-eol': (b'', MIXED_EOL),
}

# Files that are not UTF-8 text.
RAW = {
    'latin1': '# Café\n\nAccents ñ, ü.\n'.encode('latin-1'),
}


def sample_bytes(sample, with_z):
    if sample in RAW:
        return RAW[sample]
    prefix, text = SAMPLES[sample]
    if with_z:
        text += 'Z'
    return prefix + text.encode('utf-8')


out, sample = sys.argv[1], sys.argv[2]
with open(out, 'wb') as f:
    f.write(sample_bytes(sample, sys.argv[3:] == ['z']))
