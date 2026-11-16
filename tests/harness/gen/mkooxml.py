# mkooxml.py OUT KIND: a minimal OOXML package, the same bytes on every system.
# KIND is dotx, xlsx, xltx, pptx, potx or docx-truncated (the DOCX cut in half).
#   word:  one paragraph, "Harness template paragraph".
#   sheet: A1 1234.5 and B1 the text "Harness sheet text".
#   slide: one slide with a text box as large as the slide, "Harness slide text".
import sys
import zipfile

# A fixed timestamp keeps the output byte for byte the same on every run.
STAMP = (2026, 1, 1, 0, 0, 0)
# The system zipfile records in each entry: Unix, also when run on Windows.
CREATE_SYSTEM = 3

OFFICE = 'application/vnd.openxmlformats-officedocument.'
REL = 'http://schemas.openxmlformats.org/officeDocument/2006/relationships'
XML = '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n'

# KIND: (family, content type of the main part, whether to cut the file in half).
KINDS = {
    'dotx': ('word', OFFICE + 'wordprocessingml.template.main+xml', False),
    'docx-truncated': ('word', OFFICE + 'wordprocessingml.document.main+xml', True),
    'xlsx': ('sheet', OFFICE + 'spreadsheetml.sheet.main+xml', False),
    'xltx': ('sheet', OFFICE + 'spreadsheetml.template.main+xml', False),
    'pptx': ('slide', OFFICE + 'presentationml.presentation.main+xml', False),
    'potx': ('slide', OFFICE + 'presentationml.template.main+xml', False),
}


def content_types(overrides):
    lines = ''.join(f'<Override PartName="{part}" ContentType="{kind}"/>\n' for part, kind in overrides)
    return (XML + '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">\n'
            '<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>\n'
            '<Default Extension="xml" ContentType="application/xml"/>\n'
            f'{lines}</Types>')


# relationships([(type, target), ...]): a .rels part with the ids rId1, rId2...
def relationships(targets):
    lines = ''.join(f'<Relationship Id="rId{i}" Type="{REL}/{kind}" Target="{target}"/>\n'
                    for i, (kind, target) in enumerate(targets, 1))
    return (XML + '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">\n'
            f'{lines}</Relationships>')


def word_parts(main_type):
    document = (XML + '<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">'
                '<w:body><w:p><w:r><w:t>Harness template paragraph</w:t></w:r></w:p><w:sectPr/></w:body>'
                '</w:document>')
    return [
        ('[Content_Types].xml', content_types([('/word/document.xml', main_type)])),
        ('_rels/.rels', relationships([('officeDocument', 'word/document.xml')])),
        ('word/document.xml', document),
    ]


def sheet_parts(main_type):
    namespaces = ('xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" '
                  f'xmlns:r="{REL}"')
    workbook = (XML + f'<workbook {namespaces}>\n'
                '<sheets><sheet name="Sheet1" sheetId="1" r:id="rId1"/></sheets>\n'
                '</workbook>')
    sheet = (XML + f'<worksheet {namespaces}>\n<sheetData>\n'
             '<row r="1"><c r="A1"><v>1234.5</v></c>'
             '<c r="B1" t="inlineStr"><is><t>Harness sheet text</t></is></c></row>\n'
             '</sheetData>\n</worksheet>')
    types = [
        ('/xl/workbook.xml', main_type),
        ('/xl/worksheets/sheet1.xml', OFFICE + 'spreadsheetml.worksheet+xml'),
    ]
    return [
        ('[Content_Types].xml', content_types(types)),
        ('_rels/.rels', relationships([('officeDocument', 'xl/workbook.xml')])),
        ('xl/workbook.xml', workbook),
        ('xl/_rels/workbook.xml.rels', relationships([('worksheet', 'worksheets/sheet1.xml')])),
        ('xl/worksheets/sheet1.xml', sheet),
    ]


DRAWING = 'xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"'
SLIDE_NAMESPACES = (f'{DRAWING} xmlns:r="{REL}" '
                    'xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"')
# A slide of 4:3, in EMU.
SLIDE_WIDTH = 9144000
SLIDE_HEIGHT = 6858000

EMPTY_TREE = ('<p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>'
              '<p:grpSpPr/>')

# The text box covers the slide, so a click anywhere on the slide lands in it.
TEXT_BOX = ('<p:sp><p:nvSpPr><p:cNvPr id="2" name="Text"/><p:cNvSpPr txBox="1"/><p:nvPr/></p:nvSpPr>'
            f'<p:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="{SLIDE_WIDTH}" cy="{SLIDE_HEIGHT}"/></a:xfrm>'
            '<a:prstGeom prst="rect"><a:avLst/></a:prstGeom></p:spPr>'
            '<p:txBody><a:bodyPr anchor="ctr"/><a:p><a:pPr algn="ctr"/>'
            '<a:r><a:rPr lang="en-US" sz="2400"/><a:t>Harness slide text</a:t></a:r></a:p></p:txBody></p:sp>')

FILL = '<a:solidFill><a:schemeClr val="phClr"/></a:solidFill>'
LINE = f'<a:ln w="9525">{FILL}</a:ln>'
EFFECT = '<a:effectStyle><a:effectLst/></a:effectStyle>'
FONT = '<a:latin typeface="Arial"/><a:ea typeface=""/><a:cs typeface=""/>'


def theme_colours():
    values = [('dk1', '000000'), ('lt1', 'FFFFFF'), ('dk2', '1F497D'), ('lt2', 'EEECE1'),
              ('accent1', '4F81BD'), ('accent2', 'C0504D'), ('accent3', '9BBB59'), ('accent4', '8064A2'),
              ('accent5', '4BACC6'), ('accent6', 'F79646'), ('hlink', '0000FF'), ('folHlink', '800080')]
    return ''.join(f'<a:{name}><a:srgbClr val="{value}"/></a:{name}>' for name, value in values)


def theme():
    return (XML + f'<a:theme {DRAWING} name="Harness"><a:themeElements>'
            f'<a:clrScheme name="Harness">{theme_colours()}</a:clrScheme>'
            f'<a:fontScheme name="Harness"><a:majorFont>{FONT}</a:majorFont><a:minorFont>{FONT}</a:minorFont></a:fontScheme>'
            f'<a:fmtScheme name="Harness"><a:fillStyleLst>{FILL * 3}</a:fillStyleLst>'
            f'<a:lnStyleLst>{LINE * 3}</a:lnStyleLst>'
            f'<a:effectStyleLst>{EFFECT * 3}</a:effectStyleLst>'
            f'<a:bgFillStyleLst>{FILL * 3}</a:bgFillStyleLst></a:fmtScheme>'
            '</a:themeElements></a:theme>')


def slide_parts(main_type):
    presentation = (XML + f'<p:presentation {SLIDE_NAMESPACES}>'
                    '<p:sldMasterIdLst><p:sldMasterId id="2147483648" r:id="rId1"/></p:sldMasterIdLst>'
                    '<p:sldIdLst><p:sldId id="256" r:id="rId2"/></p:sldIdLst>'
                    f'<p:sldSz cx="{SLIDE_WIDTH}" cy="{SLIDE_HEIGHT}"/>'
                    f'<p:notesSz cx="{SLIDE_HEIGHT}" cy="{SLIDE_WIDTH}"/>'
                    '</p:presentation>')
    master = (XML + f'<p:sldMaster {SLIDE_NAMESPACES}>'
              f'<p:cSld><p:spTree>{EMPTY_TREE}</p:spTree></p:cSld>'
              '<p:clrMap bg1="lt1" tx1="dk1" bg2="lt2" tx2="dk2" accent1="accent1" accent2="accent2"'
              ' accent3="accent3" accent4="accent4" accent5="accent5" accent6="accent6"'
              ' hlink="hlink" folHlink="folHlink"/>'
              '<p:sldLayoutIdLst><p:sldLayoutId id="2147483649" r:id="rId1"/></p:sldLayoutIdLst>'
              '</p:sldMaster>')
    layout = (XML + f'<p:sldLayout {SLIDE_NAMESPACES} type="blank">'
              f'<p:cSld name="Blank"><p:spTree>{EMPTY_TREE}</p:spTree></p:cSld>'
              '<p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sldLayout>')
    slide = (XML + f'<p:sld {SLIDE_NAMESPACES}>'
             f'<p:cSld><p:spTree>{EMPTY_TREE}{TEXT_BOX}</p:spTree></p:cSld>'
             '<p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sld>')
    types = [
        ('/ppt/presentation.xml', main_type),
        ('/ppt/slideMasters/slideMaster1.xml', OFFICE + 'presentationml.slideMaster+xml'),
        ('/ppt/slideLayouts/slideLayout1.xml', OFFICE + 'presentationml.slideLayout+xml'),
        ('/ppt/slides/slide1.xml', OFFICE + 'presentationml.slide+xml'),
        ('/ppt/theme/theme1.xml', OFFICE + 'theme+xml'),
    ]
    presentation_rels = [
        ('slideMaster', 'slideMasters/slideMaster1.xml'),
        ('slide', 'slides/slide1.xml'),
        ('theme', 'theme/theme1.xml'),
    ]
    master_rels = [
        ('slideLayout', '../slideLayouts/slideLayout1.xml'),
        ('theme', '../theme/theme1.xml'),
    ]
    return [
        ('[Content_Types].xml', content_types(types)),
        ('_rels/.rels', relationships([('officeDocument', 'ppt/presentation.xml')])),
        ('ppt/presentation.xml', presentation),
        ('ppt/_rels/presentation.xml.rels', relationships(presentation_rels)),
        ('ppt/slideMasters/slideMaster1.xml', master),
        ('ppt/slideMasters/_rels/slideMaster1.xml.rels', relationships(master_rels)),
        ('ppt/slideLayouts/slideLayout1.xml', layout),
        ('ppt/slideLayouts/_rels/slideLayout1.xml.rels',
         relationships([('slideMaster', '../slideMasters/slideMaster1.xml')])),
        ('ppt/slides/slide1.xml', slide),
        ('ppt/slides/_rels/slide1.xml.rels', relationships([('slideLayout', '../slideLayouts/slideLayout1.xml')])),
        ('ppt/theme/theme1.xml', theme()),
    ]


FAMILIES = {'word': word_parts, 'sheet': sheet_parts, 'slide': slide_parts}


# Stored, not deflated: the bytes do not depend on the zlib of the system.
def write_package(out, parts):
    with zipfile.ZipFile(out, 'w') as z:
        for name, text in parts:
            info = zipfile.ZipInfo(name, STAMP)
            info.create_system = CREATE_SYSTEM
            z.writestr(info, text.encode('utf-8'), zipfile.ZIP_STORED)


# Leaves a file that starts like a ZIP and has no central directory.
def keep_first_half(path):
    with open(path, 'rb') as f:
        data = f.read()
    with open(path, 'wb') as f:
        f.write(data[:len(data) // 2])


out, kind = sys.argv[1], sys.argv[2]
family, main_type, truncated = KINDS[kind]
write_package(out, FAMILIES[family](main_type))
if truncated:
    keep_first_half(out)
