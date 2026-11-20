# mkxlsxstyles.py OUT: an XLSX with cell styles, the same bytes on every system.
#   Totals (new): Arial 14 bold blue, yellow fill, red bottom border, "0.00",
#                 centered, top, wrapped. Used by B1 and B2 (text).
#   Good (predefined, changed): Courier New 12 italic purple, green fill, blue
#                 outline, "#,##0.000", right, vertically centered. Used by C1.
#   Bad 2 (copy of Bad, changed): Times New Roman 11 underlined dark red,
#                 orange fill, top and bottom borders, "0%", left, wrapped.
#                 Used by D1.
#   Neutral (predefined, unchanged). Used by E1.
# A1 is 1234.5 with no style; B1:E1 are 0.25.
import sys
import zipfile

# A fixed timestamp keeps the output byte for byte the same on every run.
STAMP = (2026, 1, 1, 0, 0, 0)
# The system zipfile records in each entry: Unix, also when run on Windows.
CREATE_SYSTEM = 3

MAIN = 'http://schemas.openxmlformats.org/spreadsheetml/2006/main'
REL = 'http://schemas.openxmlformats.org/officeDocument/2006/relationships'
PKG = 'http://schemas.openxmlformats.org/package/2006/relationships'
OFFICE = 'application/vnd.openxmlformats-officedocument.spreadsheetml.'
XML = '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n'

CONTENT_TYPES = (XML + '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">\n'
                 '<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>\n'
                 '<Default Extension="xml" ContentType="application/xml"/>\n'
                 f'<Override PartName="/xl/workbook.xml" ContentType="{OFFICE}sheet.main+xml"/>\n'
                 f'<Override PartName="/xl/worksheets/sheet1.xml" ContentType="{OFFICE}worksheet+xml"/>\n'
                 f'<Override PartName="/xl/styles.xml" ContentType="{OFFICE}styles+xml"/>\n'
                 '</Types>')

RELS = (XML + f'<Relationships xmlns="{PKG}">\n'
        f'<Relationship Id="rId1" Type="{REL}/officeDocument" Target="xl/workbook.xml"/>\n'
        '</Relationships>')

WORKBOOK = (XML + f'<workbook xmlns="{MAIN}" xmlns:r="{REL}">\n'
            '<sheets><sheet name="Sheet1" sheetId="1" r:id="rId1"/></sheets>\n'
            '</workbook>')

WORKBOOK_RELS = (XML + f'<Relationships xmlns="{PKG}">\n'
                 f'<Relationship Id="rId1" Type="{REL}/worksheet" Target="worksheets/sheet1.xml"/>\n'
                 f'<Relationship Id="rId2" Type="{REL}/styles" Target="styles.xml"/>\n'
                 '</Relationships>')

TOTALS_ALIGN = '<alignment horizontal="center" vertical="top" wrapText="1"/>'
GOOD_ALIGN = '<alignment horizontal="right" vertical="center"/>'
BAD2_ALIGN = '<alignment horizontal="left" wrapText="1"/>'
APPLY_ALL = 'applyNumberFormat="1" applyFont="1" applyFill="1" applyBorder="1" applyAlignment="1"'

STYLES = (XML + f'<styleSheet xmlns="{MAIN}">\n'
          '<numFmts count="1"><numFmt numFmtId="164" formatCode="#,##0.000"/></numFmts>\n'
          '<fonts count="5">'
          '<font><sz val="11"/><color theme="1"/><name val="Calibri"/></font>'
          '<font><b/><sz val="14"/><color rgb="FF0000FF"/><name val="Arial"/></font>'
          '<font><i/><sz val="12"/><color rgb="FF800080"/><name val="Courier New"/></font>'
          '<font><u/><sz val="11"/><color rgb="FF9C0006"/><name val="Times New Roman"/></font>'
          '<font><sz val="11"/><color rgb="FF9C6500"/><name val="Calibri"/></font>'
          '</fonts>\n'
          '<fills count="6">'
          '<fill><patternFill patternType="none"/></fill>'
          '<fill><patternFill patternType="gray125"/></fill>'
          '<fill><patternFill patternType="solid"><fgColor rgb="FFFFFF00"/><bgColor indexed="64"/></patternFill></fill>'
          '<fill><patternFill patternType="solid"><fgColor rgb="FFCCFFCC"/><bgColor indexed="64"/></patternFill></fill>'
          '<fill><patternFill patternType="solid"><fgColor rgb="FFFFCC99"/><bgColor indexed="64"/></patternFill></fill>'
          '<fill><patternFill patternType="solid"><fgColor rgb="FFFFEB9C"/></patternFill></fill>'
          '</fills>\n'
          '<borders count="4">'
          '<border><left/><right/><top/><bottom/><diagonal/></border>'
          '<border><left/><right/><top/><bottom style="thin"><color rgb="FFFF0000"/></bottom><diagonal/></border>'
          '<border><left style="thin"><color rgb="FF0000FF"/></left><right style="thin"><color rgb="FF0000FF"/></right>'
          '<top style="thin"><color rgb="FF0000FF"/></top><bottom style="thin"><color rgb="FF0000FF"/></bottom><diagonal/></border>'
          '<border><left/><right/><top style="medium"><color rgb="FF9C0006"/></top>'
          '<bottom style="medium"><color rgb="FF9C0006"/></bottom><diagonal/></border>'
          '</borders>\n'
          '<cellStyleXfs count="5">'
          '<xf numFmtId="0" fontId="0" fillId="0" borderId="0"/>'
          f'<xf numFmtId="2" fontId="1" fillId="2" borderId="1" {APPLY_ALL}>{TOTALS_ALIGN}</xf>'
          f'<xf numFmtId="164" fontId="2" fillId="3" borderId="2" {APPLY_ALL}>{GOOD_ALIGN}</xf>'
          f'<xf numFmtId="9" fontId="3" fillId="4" borderId="3" {APPLY_ALL}>{BAD2_ALIGN}</xf>'
          '<xf numFmtId="0" fontId="4" fillId="5" borderId="0" applyNumberFormat="0" applyBorder="0" applyAlignment="0" applyProtection="0"/>'
          '</cellStyleXfs>\n'
          '<cellXfs count="5">'
          '<xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/>'
          f'<xf numFmtId="2" fontId="1" fillId="2" borderId="1" xfId="1" {APPLY_ALL}>{TOTALS_ALIGN}</xf>'
          f'<xf numFmtId="164" fontId="2" fillId="3" borderId="2" xfId="2" {APPLY_ALL}>{GOOD_ALIGN}</xf>'
          f'<xf numFmtId="9" fontId="3" fillId="4" borderId="3" xfId="3" {APPLY_ALL}>{BAD2_ALIGN}</xf>'
          '<xf numFmtId="0" fontId="4" fillId="5" borderId="0" xfId="4" applyFont="1" applyFill="1"/>'
          '</cellXfs>\n'
          '<cellStyles count="5">'
          '<cellStyle name="Normal" xfId="0" builtinId="0"/>'
          '<cellStyle name="Totals" xfId="1"/>'
          '<cellStyle name="Good" xfId="2" builtinId="26"/>'
          '<cellStyle name="Bad 2" xfId="3"/>'
          '<cellStyle name="Neutral" xfId="4" builtinId="28"/>'
          '</cellStyles>\n'
          '</styleSheet>')

SHEET = (XML + f'<worksheet xmlns="{MAIN}">\n<sheetData>\n'
         '<row r="1"><c r="A1"><v>1234.5</v></c><c r="B1" s="1"><v>0.25</v></c><c r="C1" s="2"><v>0.25</v></c>'
         '<c r="D1" s="3"><v>0.25</v></c><c r="E1" s="4"><v>0.25</v></c></row>\n'
         '<row r="2"><c r="B2" s="1" t="inlineStr"><is><t>Harness styled text</t></is></c></row>\n'
         '</sheetData>\n</worksheet>')

PARTS = [
    ('[Content_Types].xml', CONTENT_TYPES),
    ('_rels/.rels', RELS),
    ('xl/workbook.xml', WORKBOOK),
    ('xl/_rels/workbook.xml.rels', WORKBOOK_RELS),
    ('xl/styles.xml', STYLES),
    ('xl/worksheets/sheet1.xml', SHEET),
]


# Stored, not deflated: the bytes do not depend on the zlib of the system.
def write_package(out):
    with zipfile.ZipFile(out, 'w') as z:
        for name, text in PARTS:
            info = zipfile.ZipInfo(name, STAMP)
            info.create_system = CREATE_SYSTEM
            z.writestr(info, text.encode('utf-8'), zipfile.ZIP_STORED)


write_package(sys.argv[1])
