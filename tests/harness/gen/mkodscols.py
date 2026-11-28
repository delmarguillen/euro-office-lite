# mkodscols.py OUT: an ODS whose columns have the widths a LibreOffice sheet
# gives them, the same bytes on every system.
#   A: 0.889in, the default column of LibreOffice Calc (2.258 cm, 85 px at 96 dpi)
#   B: 1.378in (3.5 cm, 132 px)
#   C: 5cm (189 px)
# A1 is 1234.5, B1 the text "Harness ODS columns", C1 is 0.25.
# The cells are in Liberation Sans 10 pt, named the way LibreOffice 7.3 names
# a font: the family between apostrophes in fo:font-family and in the font
# face declaration (tests/corpus/basic.ods has the same lines).
import sys
import zipfile

# A fixed timestamp keeps the output byte for byte the same on every run.
STAMP = (2026, 1, 1, 0, 0, 0)
# The system zipfile records in each entry: Unix, also when run on Windows.
CREATE_SYSTEM = 3

XML = '<?xml version="1.0" encoding="UTF-8"?>\n'
ODF = 'urn:oasis:names:tc:opendocument:xmlns:'
NAMESPACES = ' '.join(f'xmlns:{prefix}="{ODF}{name}"' for prefix, name in [
    ('office', 'office:1.0'), ('style', 'style:1.0'), ('text', 'text:1.0'), ('table', 'table:1.0'),
    ('fo', 'xsl-fo-compatible:1.0'), ('svg', 'svg-compatible:1.0'),
])
MIME_TYPE = 'application/vnd.oasis.opendocument.spreadsheet'
WIDTHS = ['0.889in', '1.378in', '5cm']

FONT_FACES = ('<office:font-face-decls><style:font-face style:name="Liberation Sans"'
              ' svg:font-family="&apos;Liberation Sans&apos;" style:font-family-generic="swiss"'
              ' style:font-pitch="variable"/></office:font-face-decls>')
CELL_STYLES = ('<office:styles><style:default-style style:family="table-cell">'
               '<style:text-properties style:font-name="Liberation Sans" fo:font-size="10pt"/></style:default-style>'
               '<style:style style:name="Default" style:family="table-cell">'
               '<style:text-properties style:font-name="Liberation Sans" fo:font-family="&apos;Liberation Sans&apos;"'
               ' style:font-family-generic="swiss" style:font-pitch="variable"/></style:style></office:styles>')

COLUMN_STYLES = ''.join(
    f'<style:style style:name="co{n}" style:family="table-column">'
    f'<style:table-column-properties fo:break-before="auto" style:column-width="{width}"/></style:style>'
    for n, width in enumerate(WIDTHS, 1))
COLUMNS = ''.join(f'<table:table-column table:style-name="co{n}" table:default-cell-style-name="Default"/>' for n in range(1, len(WIDTHS) + 1))

CONTENT = (XML + f'<office:document-content {NAMESPACES} office:version="1.2">'
           f'{FONT_FACES}<office:automatic-styles>{COLUMN_STYLES}</office:automatic-styles>'
           '<office:body><office:spreadsheet><table:table table:name="Sheet1">'
           f'{COLUMNS}<table:table-row>'
           '<table:table-cell office:value-type="float" office:value="1234.5"><text:p>1234.5</text:p></table:table-cell>'
           '<table:table-cell office:value-type="string"><text:p>Harness ODS columns</text:p></table:table-cell>'
           '<table:table-cell office:value-type="float" office:value="0.25"><text:p>0.25</text:p></table:table-cell>'
           '</table:table-row></table:table></office:spreadsheet></office:body></office:document-content>')

STYLES = (XML + f'<office:document-styles {NAMESPACES} office:version="1.2">'
          f'{FONT_FACES}{CELL_STYLES}</office:document-styles>')

MANIFEST = (XML + f'<manifest:manifest xmlns:manifest="{ODF}manifest:1.0" manifest:version="1.2">\n'
            + ''.join(f'<manifest:file-entry manifest:full-path="{path}" manifest:media-type="{kind}"/>\n'
                      for path, kind in [('/', MIME_TYPE), ('content.xml', 'text/xml'), ('styles.xml', 'text/xml')])
            + '</manifest:manifest>')

# Stored, not deflated: the bytes do not depend on the zlib of the system, and
# ODF wants its first entry, mimetype, stored.
with zipfile.ZipFile(sys.argv[1], 'w') as z:
    for name, text in [('mimetype', MIME_TYPE), ('META-INF/manifest.xml', MANIFEST),
                       ('content.xml', CONTENT), ('styles.xml', STYLES)]:
        info = zipfile.ZipInfo(name, STAMP)
        info.create_system = CREATE_SYSTEM
        z.writestr(info, text.encode('utf-8'), zipfile.ZIP_STORED)
