# mkdocxtoc.py OUT: a DOCX with fields whose stored results are stale, the
# same bytes on every system.
#   Section 1, pages in lower-case roman numbers from i:
#     a table of contents (TOC \o "1-1" \h), stored result "STALE TOC";
#     a page break; the heading "Preface heading" (Heading 1, numbered "1").
#   Section 2, pages in arabic numbers from 1:
#     the heading "Main heading" (Heading 1, numbered "2");
#     a caption "Figure " STYLEREF 1 \s "-" SEQ Figure \* ARABIC \s 1,
#     stored results "9" and "9".
# After every field is updated the table of contents reads
# "1 Preface heading ... ii" and "2 Main heading ... 1", and the caption
# "Figure 2-1".
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
                 f'<Override PartName="/word/styles.xml" ContentType="{WORD}styles+xml"/>\n'
                 f'<Override PartName="/word/numbering.xml" ContentType="{WORD}numbering+xml"/>\n'
                 '</Types>')

RELS = (XML + f'<Relationships xmlns="{PKG}">\n'
        f'<Relationship Id="rId1" Type="{REL}/officeDocument" Target="word/document.xml"/>\n'
        '</Relationships>')

DOCUMENT_RELS = (XML + f'<Relationships xmlns="{PKG}">\n'
                 f'<Relationship Id="rId1" Type="{REL}/styles" Target="styles.xml"/>\n'
                 f'<Relationship Id="rId2" Type="{REL}/numbering" Target="numbering.xml"/>\n'
                 '</Relationships>')

STYLES = (XML + f'<w:styles xmlns:w="{W}">'
          '<w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/></w:style>'
          '<w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="heading 1"/><w:basedOn w:val="Normal"/>'
          '<w:next w:val="Normal"/><w:pPr><w:keepNext/><w:numPr><w:ilvl w:val="0"/><w:numId w:val="1"/></w:numPr>'
          '<w:outlineLvl w:val="0"/></w:pPr><w:rPr><w:b/><w:sz w:val="32"/></w:rPr></w:style>'
          '<w:style w:type="paragraph" w:styleId="TOC1"><w:name w:val="toc 1"/><w:basedOn w:val="Normal"/>'
          '<w:next w:val="Normal"/></w:style>'
          '</w:styles>')

NUMBERING = (XML + f'<w:numbering xmlns:w="{W}">'
             '<w:abstractNum w:abstractNumId="0"><w:multiLevelType w:val="multilevel"/>'
             '<w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="decimal"/><w:pStyle w:val="Heading1"/>'
             '<w:lvlText w:val="%1"/><w:lvlJc w:val="left"/></w:lvl></w:abstractNum>'
             '<w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num>'
             '</w:numbering>')


def run(text):
    return f'<w:r><w:t xml:space="preserve">{text}</w:t></w:r>'


# A complex field: its instruction, then the result stored in the file.
def field(instruction, result):
    return ('<w:r><w:fldChar w:fldCharType="begin"/></w:r>'
            f'<w:r><w:instrText xml:space="preserve"> {instruction} </w:instrText></w:r>'
            '<w:r><w:fldChar w:fldCharType="separate"/></w:r>'
            f'{run(result)}<w:r><w:fldChar w:fldCharType="end"/></w:r>')


def heading(text):
    return f'<w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr>{run(text)}</w:p>'


PAGE = '<w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440"/>'
ROMAN_SECTION = f'<w:sectPr>{PAGE}<w:pgNumType w:fmt="lowerRoman" w:start="1"/></w:sectPr>'
ARABIC_SECTION = f'<w:sectPr>{PAGE}<w:pgNumType w:fmt="decimal" w:start="1"/></w:sectPr>'

TOC = field('TOC \\o &quot;1-1&quot; \\h', 'STALE TOC')
CHAPTER = field('STYLEREF 1 \\s', '9')
FIGURE = field('SEQ Figure \\* ARABIC \\s 1', '9')

BODY = (f'<w:p>{TOC}</w:p>'
        '<w:p><w:r><w:br w:type="page"/></w:r></w:p>'
        + heading('Preface heading')
        + f'<w:p><w:pPr>{ROMAN_SECTION}</w:pPr>{run("End of the preface.")}</w:p>'
        + heading('Main heading')
        + f'<w:p>{run("Figure ")}{CHAPTER}{run("-")}{FIGURE}</w:p>'
        + ARABIC_SECTION)

DOCUMENT = XML + f'<w:document xmlns:w="{W}"><w:body>{BODY}</w:body></w:document>'

# Stored, not deflated: the bytes do not depend on the zlib of the system.
with zipfile.ZipFile(sys.argv[1], 'w') as z:
    for name, text in [('[Content_Types].xml', CONTENT_TYPES), ('_rels/.rels', RELS),
                       ('word/document.xml', DOCUMENT), ('word/_rels/document.xml.rels', DOCUMENT_RELS),
                       ('word/styles.xml', STYLES), ('word/numbering.xml', NUMBERING)]:
        info = zipfile.ZipInfo(name, STAMP)
        info.create_system = CREATE_SYSTEM
        z.writestr(info, text.encode('utf-8'), zipfile.ZIP_STORED)
