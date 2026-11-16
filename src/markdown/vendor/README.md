# Markdown page libraries

The page loads nothing from the network. The first four are copied unchanged
from the npm packages below (`npm pack <name>@<version>`). Only `markdown-it-task-lists.js` adds
two wrapper lines around the package's `index.js`, which is CommonJS only.

| File | Package | Version | License | Bytes | Gzip | sha256 |
|---|---|---|---|---|---|---|
| `markdown-it.min.js` | markdown-it (`dist/browser/markdown-it.umd.min.js`) | 15.0.3 | MIT | 100786 | 41279 | `391f2aa638e75e6c7859d0a5c5650ef0956722a45998f17e05b83bc655778e1e` |
| `markdown-it-footnote.min.js` | markdown-it-footnote (`dist/markdown-it-footnote.min.js`) | 4.0.0 | MIT | 5624 | 1850 | `d6fee58a3b56c5742fa18f3e01f1d317cc99975683ebd39c9195cb2aff0c2e42` |
| `markdown-it-task-lists.js` | @hackmd/markdown-it-task-lists (`index.js`) | 2.1.4 | ISC | 4368 | 1535 | `eabf5f8049272c65c1c9dbc809a0b61400dbed82568ecaaddd85e7d2a8763c1c` |
| `purify.min.js` | dompurify (`dist/purify.min.js`) | 3.4.17 | MPL-2.0 OR Apache-2.0 | 29828 | 11757 | `cd37898f9782adf54820c413191b5691803f0f137c799d6edcfd38523b4c2c31` |
| `codemirror.min.js` | CodeMirror 6 bundle, see below | | MIT | 543127 | 185185 | `75f21998c3832416e833c01d18dad17e63111a7595c0f998bd0882f32eb70d6b` |

The license of each is next to it (`LICENSE-*.txt`).

To update one: `npm pack <name>@<version>`, copy the same file from the
tarball, and update this table.

## codemirror.min.js

The editor, loaded only when Edit is pressed. Built, not copied:
`scripts/vendor-codemirror.sh` runs `npm ci` in `tools/codemirror-vendor/`
(versions pinned by its own `package.json` and `package-lock.json`, apart
from the app's) and bundles `tools/codemirror-vendor/entry.mjs` with esbuild
0.28.2 into one minified IIFE. Running it again gives the same sha256.
Licenses: `LICENSE-codemirror.txt`.

The packages in the bundle. "Alone" is each package minified by itself, its
own dependencies left out; "In bundle" is what esbuild kept of it.

| Package | Version | License | Alone min | Alone gzip | In bundle |
|---|---|---|---|---|---|
| @codemirror/view | 6.43.15 | MIT | 199230 | 63968 | 171183 |
| @codemirror/state | 6.7.6 | MIT | 47509 | 15575 | 47151 |
| @codemirror/commands | 6.11.1 | MIT | 29321 | 9678 | 23214 |
| @codemirror/language | 6.13.1 | MIT | 36861 | 13264 | 18586 |
| @codemirror/search | 6.7.2 | MIT | 21105 | 7213 | 18536 |
| @codemirror/lang-markdown | 6.5.2 | MIT | 9096 | 3942 | 8239 |
| @codemirror/lang-html (from lang-markdown) | 6.4.12 | MIT | 14557 | 5547 | 14042 |
| @codemirror/lang-css (from lang-html) | 6.3.1 | MIT | 11225 | 4602 | 10936 |
| @codemirror/lang-javascript (from lang-html) | 6.2.5 | MIT | 9492 | 3932 | 6540 |
| @codemirror/autocomplete (from lang-markdown) | 6.20.4 | MIT | 38360 | 13535 | 8440 |
| @lezer/javascript | 1.5.6 | MIT | 77861 | 29711 | 77770 |
| @lezer/markdown | 1.8.1 | MIT | 36621 | 12287 | 36316 |
| @lezer/common | 1.5.5 | MIT | 28024 | 9633 | 27967 |
| @lezer/lr | 1.4.12 | MIT | 27127 | 8993 | 26985 |
| @lezer/css | 1.3.9 | MIT | 17216 | 7939 | 17154 |
| @lezer/html | 1.3.13 | MIT | 14449 | 6067 | 14325 |
| @lezer/highlight | 1.2.5 | MIT | 7374 | 2993 | 7084 |
| @marijn/find-cluster-break | 1.0.4 | MIT | 2347 | 1217 | 2329 |
| style-mod | 4.1.4 | MIT | 2233 | 1164 | 2211 |
| w3c-keyname | 2.2.8 | MIT | 1515 | 810 | 1516 |
| crelt | 1.0.7 | MIT | 629 | 386 | 611 |

lang-markdown always brings lang-html (for HTML inside Markdown), and with it
the CSS and JavaScript parsers: about 150 KB of the bundle. Completion is
turned off (`completeHTMLTags: false`); autocomplete is in only as their
dependency. @codemirror/lint and @codemirror/streamparser are installed as
dependencies but nothing of them is bundled.

To update: change the versions in `tools/codemirror-vendor/package.json`, run
`npm install` there to update the lock file, run the script, and update both
tables.
