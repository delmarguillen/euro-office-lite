# Writes the CSV and TXT fixtures byte for byte into the folder given as the
# only argument. Each one is one case of the text import and save path.
import os
import sys

out = sys.argv[1]
BOM8 = b"\xef\xbb\xbf"


def write(name, data):
    with open(os.path.join(out, name), "wb") as f:
        f.write(data)


write("utf8-bom-semicolon-crlf.csv",
      BOM8 + "Name;Price;Quantity\r\nCafé;1,50;3\r\nTé verde;2,25;10\r\n".encode("utf-8"))
write("utf8-comma-lf-final.csv",
      "name,city,amount\nAna,Málaga,10\nBjörn,Göteborg,20\n".encode("utf-8"))
write("utf8-comma-nofinal.csv", b"name,amount\nfirst,1\nsecond,2")
write("win1252-semicolon.csv",
      "Año;Ciudad;Importe\r\n2024;Córdoba;1.234,50\r\nNiño;Peñíscola;7\r\n".encode("cp1252"))
write("utf16le-tab.csv",
      b"\xff\xfe" + "col1\tcol2\tcol3\r\nalpha α\tbeta β\tgamma γ\r\n1\t2\t3\r\n".encode("utf-16-le"))
write("quoted-delimiter.csv",
      b'id;text;note\n1;"a;b";"say ""hi"""\n2;"semi;colon";plain\n')
write("empty-lines.csv", b"a,b\n1,2\n\n3,4\n")
write("utf8.txt", "First line\nSegunda línea con ñ\n".encode("utf-8"))
write("win1252-crlf.txt", "Línea uno\r\nCafé € ‰\r\n".encode("cp1252"))
write("utf16be.txt", b"\xfe\xff" + "Hello\nÜber alles\n".encode("utf-16-be"))
