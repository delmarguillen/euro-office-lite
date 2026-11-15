# mkdocx.py OUT PARAGRAPHS: a minimal DOCX whose paragraph i reads "Paragraph i: ...".
import sys
import zipfile

# A fixed timestamp keeps the output byte for byte the same on every run.
STAMP = (2026, 1, 1, 0, 0, 0)

CONTENT_TYPES = '''<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
<Default Extension="xml" ContentType="application/xml"/>
<Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>
</Types>'''

RELS = '''<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>
</Relationships>'''


def paragraph(i):
    text = f'Paragraph {i}: the quick brown fox jumps over the lazy dog.'
    return f'<w:p><w:r><w:t>{text}</w:t></w:r></w:p>'


def document(count):
    body = ''.join(paragraph(i) for i in range(count))
    return ('<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
            '<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">'
            f'<w:body>{body}<w:sectPr/></w:body></w:document>')


def add(z, name, text):
    z.writestr(zipfile.ZipInfo(name, STAMP), text, zipfile.ZIP_DEFLATED)


out, count = sys.argv[1], int(sys.argv[2])
with zipfile.ZipFile(out, 'w') as z:
    add(z, '[Content_Types].xml', CONTENT_TYPES)
    add(z, '_rels/.rels', RELS)
    add(z, 'word/document.xml', document(count))
