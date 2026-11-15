# mktxt.py OUT SAMPLE: writes one of the text samples below, byte for byte.
import sys

SAMPLES = {
    # A character above U+FFFF in the middle line.
    'emoji-utf8': 'x\na \U0001F600 b\ny\n'.encode('utf-8'),
}

out, sample = sys.argv[1], sys.argv[2]
with open(out, 'wb') as f:
    f.write(SAMPLES[sample])
