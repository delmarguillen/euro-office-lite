# mkmd.py OUT SAMPLE [z]: writes one of the Markdown samples below, byte for
# byte. With "z", writes what saving the sample after typing Z at the end of
# the document gives: Ctrl+End in the Markdown editor goes past the final
# line break, so the Z comes after it.
import sys

BOM = b'\xef\xbb\xbf'
TEXT = '# Notes\n\nAccents áéíóú ñ, Cyrillic Привет.\n\n- one\n- two\n'
# An emoji and a CJK Extension B character, both above U+FFFF.
EMOJI = '# Emoji\n\nsmile \U0001F600 and \U00020000\n'

SAMPLES = {
    'utf8': (b'', TEXT),
    'utf8-bom': (BOM, TEXT),
    'crlf': (b'', TEXT.replace('\n', '\r\n')),
    'emoji': (b'', EMOJI),
}

out, sample = sys.argv[1], sys.argv[2]
prefix, text = SAMPLES[sample]
if sys.argv[3:] == ['z']:
    text += 'Z'
with open(out, 'wb') as f:
    f.write(prefix + text.encode('utf-8'))
