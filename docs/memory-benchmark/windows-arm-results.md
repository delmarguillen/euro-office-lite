# Memory Benchmark on Windows ARM64

Measured on 2026-07-18 using Euro-Office Lite 0.16.5-alpha on a Microsoft Surface Pro 9 with Windows 11 Home 10.0.26200, ARM64, 15.4 GiB RAM, and 8 logical processors. The repository checkout at the time of the test was branch `feat/34-production-frontend` at commit `825713a`; the installed executable did not expose file-version metadata.

The application was installed under the current user's local application directory and runs on WebView2 as a seven-process tree: the Tauri/Rust application, the WebView2 host, renderer and GPU processes, two utility processes, and crashpad.

Measurements were taken from the complete descendant process tree. `Working Set` is the sum of resident memory reported for each process and may double-count pages shared by WebView2 processes. `Private` is the sum of private committed memory and is the more useful Windows measurement for comparing the application's incremental footprint. These values must not be compared directly with the Linux PSS measurements in `ubuntu-results.md`.

CPU was calculated from the change in cumulative processor time over five-second intervals. The CPU figures below are percentages of the whole eight-logical-processor machine; per-process sampling originally expressed as a percentage of one logical processor was divided by eight.

## Test sequence

The application was allowed to settle on the start screen. A blank text document was then created, closed back to the start screen, created a second time, and closed again. Each state was sampled repeatedly at five-second intervals without typing in the document.

| State | Working Set | Private | Whole-machine CPU | Notes |
|---|---:|---:|---:|---|
| Cold start screen | 435-463 MiB | 172-200 MiB | 0-0.4% | Seven processes; stable idle baseline |
| First blank document | about 1,020 MiB | about 615 MiB | 0.7-1.1% | Late peak reached 1,114 MiB working / 709 MiB private, primarily in GPU |
| Start screen after first close | 562 MiB | 355.5 MiB | 0-0.04% | Stable for 20 seconds |
| Second blank document | 809-821 MiB | 498-510 MiB | 0.15-0.66% | Lower than the first load; previously loaded resources were reused |
| Start screen after second close | 672 MiB | 377.2 MiB | 0-0.24%, ending at 0% | Fell from 456 MiB to 377 MiB private during the sample window |

## Process observations

On the cold start screen, the native application process used about 46 MiB working / 5.5 MiB private. The renderer used about 76 MiB working / 32 MiB private, while the GPU process varied between about 100-128 MiB working / 64-93 MiB private.

With the first blank document loaded and settled, the renderer used about 376 MiB working / 287 MiB private and the GPU process about 326 MiB working / 243 MiB private. The native application remained comparatively small at about 95 MiB working / 13 MiB private. Most of the editor cost therefore resides in the WebView2 renderer and GPU processes rather than in the Tauri/Rust shell.

After the first close, the renderer retained about 135 MiB private and the GPU process about 136 MiB private. After the second close, they retained about 162 MiB and 131 MiB private respectively. Some editor and graphics resources therefore remain cached after returning to the start screen.

## Findings

- Opening the first blank text document adds roughly 415 MiB of private committed memory over the cold start-screen baseline.
- Closing it releases about 260 MiB private, but the warm start screen retains approximately 155-184 MiB above the cold baseline.
- The second editor load is cheaper: it adds about 143 MiB private over the first warm start screen and stabilizes about 105 MiB below the first editor load. This is consistent with reuse of the loaded editor engine and graphics caches.
- Private memory after the second close is 21.7 MiB (6.1%) above the first post-close result. Handles fell from 3,590 after the first close to 3,457 after the second, and CPU returned to zero. Across these two cycles there is no evidence of a monotonic memory or handle leak.
- The higher final Working Set is not, by itself, evidence of a leak: Windows can leave cached or shared WebView2 pages resident until there is memory pressure. Private memory and repeated open/close cycles are more useful indicators.
- The main runtime optimization target is the editor's renderer/GPU footprint and the resources retained after its first use. Production asset pruning and Rust binary-size settings primarily reduce the package and executable size; assets that are never loaded do not materially reduce this WebView2 runtime baseline.

## Interpretation and next measurements

This short two-cycle test supports a warm-cache explanation rather than an accumulating leak. It does not prove that longer sessions are leak-free. A follow-up benchmark should repeat at least five open/close cycles and include representative DOCX, XLSX, and PPTX files, recording the post-close private-memory floor after each cycle.

For a cross-platform comparison, repeat the same state sequence on Linux and compare Windows private memory with Linux USS/private memory. Linux PSS can additionally show the fair share of common libraries, but there is no exact one-to-one mapping between PSS and the summed Windows Working Set.

## Measurement context note

These measurements were taken on the daily work machine with browser tabs open, reflecting typical everyday usage rather than an idle, dedicated test environment.
