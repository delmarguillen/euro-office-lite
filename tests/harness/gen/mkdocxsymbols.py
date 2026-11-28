# mkdocxsymbols.py OUT [CODEPOINT...]: a DOCX in Liberation Sans, the same
# bytes on every system: "Symbols: " U+263A U+2192 U+2713, "Emoji: " U+1F600
# U+1F44D and, when any is given, "Extra: " and each CODEPOINT (hexadecimal).
import sys
import zipfile

# A fixed timestamp keeps the output byte for byte the same on every run.
STAMP = (2026, 1, 1, 0, 0, 0)
# The system zipfile records in each entry: Unix, also when run on Windows.
CREATE_SYSTEM = 3

W = 'http://schemas.openxmlformats.org/wordprocessingml/2006/main'
PKG = 'http://schemas.openxmlformats.org/package/2006/relationships'
REL = 'http://schemas.openxmlformats.org/officeDocument/2006/relationships'
WORD = 'application/vnd.openxmlformats-officedocument.wordprocessingml.'
XML = '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n'

CONTENT_TYPES = (XML + '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">\n'
                 '<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>\n'
                 '<Default Extension="xml" ContentType="application/xml"/>\n'
                 f'<Override PartName="/word/document.xml" ContentType="{WORD}document.main+xml"/>\n'
                 '</Types>')

RELS = (XML + f'<Relationships xmlns="{PKG}">\n'
        f'<Relationship Id="rId1" Type="{REL}/officeDocument" Target="word/document.xml"/>\n'
        '</Relationships>')

FONT = ('<w:rPr><w:rFonts w:ascii="Liberation Sans" w:hAnsi="Liberation Sans" w:cs="Liberation Sans"/>'
        '<w:sz w:val="28"/></w:rPr>')


def paragraph(label, code_points):
    text = label + ' ' + ' '.join(chr(point) for point in code_points)
    return f'<w:p><w:r>{FONT}<w:t xml:space="preserve">{text}</w:t></w:r></w:p>'


def document(extra):
    body = paragraph('Symbols:', [0x263A, 0x2192, 0x2713]) + paragraph('Emoji:', [0x1F600, 0x1F44D])
    if extra:
        body += paragraph('Extra:', extra)
    return XML + f'<w:document xmlns:w="{W}"><w:body>{body}<w:sectPr/></w:body></w:document>'


extra = [int(word, 16) for word in sys.argv[2:]]
# Stored, not deflated: the bytes do not depend on the zlib of the system.
with zipfile.ZipFile(sys.argv[1], 'w') as z:
    for name, text in [('[Content_Types].xml', CONTENT_TYPES), ('_rels/.rels', RELS),
                       ('word/document.xml', document(extra))]:
        info = zipfile.ZipInfo(name, STAMP)
        info.create_system = CREATE_SYSTEM
        z.writestr(info, text.encode('utf-8'), zipfile.ZIP_STORED)
