# Markdown page libraries

Copied unchanged from the npm packages below (`npm pack <name>@<version>`), so
the page loads nothing from the network. Only `markdown-it-task-lists.js` adds
two wrapper lines around the package's `index.js`, which is CommonJS only.

| File | Package | Version | License | Bytes | Gzip |
|---|---|---|---|---|---|
| `markdown-it.min.js` | markdown-it (`dist/browser/markdown-it.umd.min.js`) | 15.0.3 | MIT | 100786 | 41279 |
| `markdown-it-footnote.min.js` | markdown-it-footnote (`dist/markdown-it-footnote.min.js`) | 4.0.0 | MIT | 5624 | 1850 |
| `markdown-it-task-lists.js` | @hackmd/markdown-it-task-lists (`index.js`) | 2.1.4 | ISC | 4368 | 1535 |
| `purify.min.js` | dompurify (`dist/purify.min.js`) | 3.4.17 | MPL-2.0 OR Apache-2.0 | 29828 | 11757 |

The license of each is next to it (`LICENSE-*.txt`).

To update one: `npm pack <name>@<version>`, copy the same file from the
tarball, and update this table.
