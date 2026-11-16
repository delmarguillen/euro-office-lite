# mkcfb.py OUT: a CFB (compound file, version 3) whose root holds the two
# streams of an OOXML document with a password: "EncryptionInfo" and
# "EncryptedPackage". The streams hold filler, not a real encryption: no
# password opens it. It is the layout a reader tells such a document by.
import hashlib
import struct
import sys

SECTOR = 512
MINI_SECTOR = 64
MINI_STREAM_CUTOFF = 4096
FREE = 0xFFFFFFFF
END_OF_CHAIN = 0xFFFFFFFE
FAT_SECTOR = 0xFFFFFFFD
NONE = 0xFFFFFFFF
STREAM = 2
ROOT = 5
RED = 0
BLACK = 1

# Sector numbers. The header takes the first 512 bytes and is not a sector.
FAT_AT = 0
DIRECTORY_AT = 1
MINI_FAT_AT = 2
MINI_STREAM_AT = 3
PACKAGE_AT = 4


def padded(data, size):
    return data + bytes(-len(data) % size)


# The version (4.4, agile) and flags of an EncryptionInfo, then an empty
# descriptor: under the cutoff, so it lives in the mini stream.
def encryption_info():
    descriptor = '<encryption xmlns="http://schemas.microsoft.com/office/2006/encryption"/>'
    return struct.pack('<HHI', 4, 4, 0x40) + descriptor.encode('utf-8')


# The length of the package it claims to hold, then filler up to the cutoff, so
# it lives in sectors of its own.
def encrypted_package():
    filler = b''
    block = b'harness'
    while len(filler) < MINI_STREAM_CUTOFF - 8:
        block = hashlib.sha256(block).digest()
        filler += block
    return struct.pack('<Q', 4000) + filler[:MINI_STREAM_CUTOFF - 8]


# chain(first, count): the table entries of count sectors in a row from first.
def chain(first, count):
    return list(range(first + 1, first + count)) + [END_OF_CHAIN]


def table(entries, size):
    entries = entries + [FREE] * (size - len(entries))
    return struct.pack(f'<{size}I', *entries)


def entry(name, kind, colour, left, right, child, start, size):
    encoded = (name + '\0').encode('utf-16-le')
    return (padded(encoded, 64) + struct.pack('<HBB', len(encoded), kind, colour)
            + struct.pack('<III', left, right, child) + bytes(36)
            + struct.pack('<IQ', start, size))


def unused_entry():
    return bytes(68) + struct.pack('<III', NONE, NONE, NONE) + bytes(48)


def header():
    difat = table([FAT_AT], 109)
    return (bytes([0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1]) + bytes(16)
            # Minor and major version, little endian mark, sector sizes as powers of 2.
            + struct.pack('<HHHHH', 0x3E, 3, 0xFFFE, 9, 6) + bytes(6)
            # Directory sectors (0 in version 3), FAT sectors, first directory sector.
            + struct.pack('<III', 0, 1, DIRECTORY_AT)
            + struct.pack('<II', 0, MINI_STREAM_CUTOFF)
            # First mini FAT sector and their number, first DIFAT sector and their number.
            + struct.pack('<IIII', MINI_FAT_AT, 1, END_OF_CHAIN, 0)
            + difat)


info = encryption_info()
package = encrypted_package()
mini_stream = padded(info, MINI_SECTOR)
info_mini_sectors = len(mini_stream) // MINI_SECTOR
package_sectors = len(padded(package, SECTOR)) // SECTOR

fat = table([FAT_SECTOR, END_OF_CHAIN, END_OF_CHAIN, END_OF_CHAIN] + chain(PACKAGE_AT, package_sectors),
            SECTOR // 4)
mini_fat = table(chain(0, info_mini_sectors), SECTOR // 4)
# Names sort by length first: EncryptionInfo is the root of the tree and
# EncryptedPackage its right child.
directory = (entry('Root Entry', ROOT, BLACK, NONE, NONE, 1, MINI_STREAM_AT, len(mini_stream))
             + entry('EncryptionInfo', STREAM, BLACK, NONE, 2, NONE, 0, len(info))
             + entry('EncryptedPackage', STREAM, RED, NONE, NONE, NONE, PACKAGE_AT, len(package))
             + unused_entry())

with open(sys.argv[1], 'wb') as f:
    f.write(header())
    f.write(fat)
    f.write(directory)
    f.write(mini_fat)
    f.write(padded(mini_stream, SECTOR))
    f.write(padded(package, SECTOR))
