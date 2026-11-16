# mkodf.py OUT KIND: a minimal ODF package, the same bytes on every system.
# KIND is odt (one paragraph, "Harness ODT text"), ods (A1 1234.5 and B1 the
# text "Harness ODS text") or odp (one page with a text box as large as the
# page, "Harness ODP text").
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
    ('draw', 'drawing:1.0'), ('presentation', 'presentation:1.0'),
    ('fo', 'xsl-fo-compatible:1.0'), ('svg', 'svg-compatible:1.0'),
])

MIME_TYPES = {
    'odt': 'application/vnd.oasis.opendocument.text',
    'ods': 'application/vnd.oasis.opendocument.spreadsheet',
    'odp': 'application/vnd.oasis.opendocument.presentation',
}

BODIES = {
    'odt': '<office:text><text:p>Harness ODT text</text:p></office:text>',
    'ods': ('<office:spreadsheet><table:table table:name="Sheet1"><table:table-column/><table:table-row>'
            '<table:table-cell office:value-type="float" office:value="1234.5"><text:p>1234.5</text:p></table:table-cell>'
            '<table:table-cell office:value-type="string"><text:p>Harness ODS text</text:p></table:table-cell>'
            '</table:table-row></table:table></office:spreadsheet>'),
    'odp': ('<office:presentation><draw:page draw:name="page1" draw:master-page-name="Default">'
            '<draw:frame svg:x="0cm" svg:y="0cm" svg:width="25.4cm" svg:height="19.05cm"><draw:text-box>'
            '<text:p>Harness ODP text</text:p></draw:text-box></draw:frame>'
            '</draw:page></office:presentation>'),
}

# The page an ODP needs for its master page; a text or a sheet needs none.
PAGE_STYLES = {
    'odt': '',
    'ods': '',
    'odp': ('<office:automatic-styles><style:page-layout style:name="PM1">'
            '<style:page-layout-properties fo:page-width="25.4cm" fo:page-height="19.05cm"'
            ' style:print-orientation="landscape"/></style:page-layout></office:automatic-styles>'
            '<office:master-styles><style:master-page style:name="Default" style:page-layout-name="PM1"/>'
            '</office:master-styles>'),
}


def manifest(mime_type):
    entries = [('/', mime_type), ('content.xml', 'text/xml'), ('styles.xml', 'text/xml')]
    lines = ''.join(f'<manifest:file-entry manifest:full-path="{path}" manifest:media-type="{kind}"/>\n'
                    for path, kind in entries)
    return (XML + f'<manifest:manifest xmlns:manifest="{ODF}manifest:1.0" manifest:version="1.2">\n'
            f'{lines}</manifest:manifest>')


def content(kind):
    return (XML + f'<office:document-content {NAMESPACES} office:version="1.2">'
            f'<office:body>{BODIES[kind]}</office:body></office:document-content>')


def styles(kind):
    return (XML + f'<office:document-styles {NAMESPACES} office:version="1.2">'
            f'{PAGE_STYLES[kind]}</office:document-styles>')


# Stored, not deflated: the bytes do not depend on the zlib of the system, and
# ODF wants its first entry, mimetype, stored.
def write_package(out, parts):
    with zipfile.ZipFile(out, 'w') as z:
        for name, text in parts:
            info = zipfile.ZipInfo(name, STAMP)
            info.create_system = CREATE_SYSTEM
            z.writestr(info, text.encode('utf-8'), zipfile.ZIP_STORED)


out, kind = sys.argv[1], sys.argv[2]
write_package(out, [
    ('mimetype', MIME_TYPES[kind]),
    ('META-INF/manifest.xml', manifest(MIME_TYPES[kind])),
    ('content.xml', content(kind)),
    ('styles.xml', styles(kind)),
])
