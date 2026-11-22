# Test fixtures

Real files for the `content_matches_extension` tests in `src-tauri/src/x2t_task.rs` and the package tests in `src-tauri/src/converter.rs`.

| File | What it is | Source | License | sha256 |
|---|---|---|---|---|
| `word97.doc` | Word 97-2003 document (CFB, `Word.Document.8`, 9728 bytes). Text: "Before text; Hyperlink text; after text". No author in its metadata. | Apache POI `test-data/document/hyperlink.doc`, commit [`521b26f`](https://github.com/apache/poi/blob/521b26fad979c1992538584c199fc75f2a1fd7aa/test-data/document/hyperlink.doc), copied unchanged | Apache-2.0 | `090c1e410cf91552c91e50585031900c48c659fbb9efe54bf12a35137b86b7e6` |
| `encrypted-test.docx` | Password-protected `.docx` (CFB with `EncryptionInfo` and `EncryptedPackage`, agile encryption, 41984 bytes). Decrypted text: "Encrypted fixture for euro-office-lite tests." | Made for this repo: python-docx 1.2.0, then `msoffcrypto-tool -e -p test` (6.0.0) | AGPL-3.0, same as this repo | `301dffdb1fc7f1fbbbb768218a99369d1278a6dfc5b896951c0daec132c280be` |
| `libreoffice-custom-xml.xlsx` | XLSX saved by LibreOffice (6387 bytes) with the part `customXml/item1.xml`, which `_rels/.rels` names as `../customXml/item1.xml`. Cells: A1 "customXml", B1 42. No author in its metadata. | Made for this repo: a hand-written XLSX with that part, saved with `soffice --headless --convert-to xlsx` (LibreOffice 24.2.7.2) | AGPL-3.0, same as this repo | `adf78967c1f2021b055f6c78ca226da27bb65dba86568e3ae78a4bae71f4bf37` |

Password of `encrypted-test.docx`: `test`.

Check it: `msoffcrypto-tool -p test encrypted-test.docx out.docx`.
