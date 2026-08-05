# Memory Benchmark on Ubuntu Linux

Measured on 2026-06-28 using Euro-Office Lite 0.6.0-alpha on a bare-metal Ubuntu 22.04.5 LTS machine with 16 GB RAM, 8 CPU cores, and kernel 6.8.0-124-generic.

The app runs as three processes: the main Tauri/Rust process, a WebKitWebProcess that hosts the sdkjs editor, and a WebKitNetworkProcess. All measurements use PSS (Proportional Set Size) from /proc/PID/smaps, which distributes shared library memory fairly across processes instead of double-counting it like RSS does.

## Idle (start screen, no document)

| Process | RSS | PSS | Private |
|---|---|---|---|
| euro-office-lite (Tauri/Rust) | 170 MB | 89 MB | 57 MB |
| WebKitWebProcess | 185 MB | 108 MB | 78 MB |
| WebKitNetworkProcess | 52 MB | 18 MB | 10 MB |
| Total | 407 MB | 214 MB | 145 MB |

## Editing (blank .docx, writer editor loaded)

| Process | RSS | PSS | Private |
|---|---|---|---|
| euro-office-lite (Tauri/Rust) | 253 MB | 172 MB | 140 MB |
| WebKitWebProcess | 564 MB | 485 MB | 455 MB |
| WebKitNetworkProcess | 52 MB | 18 MB | 10 MB |
| Total | 870 MB | 675 MB | 605 MB |

## Notes

These numbers reflect the full vanilla Euro-Office editor bundle. I haven't stripped or optimized anything from the upstream codebase yet, so there is room for improvement. The jump from idle to editing is almost entirely in WebKitWebProcess, which loads the complete sdkjs word editor, canvas rendering, and font subsystem.

The WebKitNetworkProcess stays flat at around 18 MB since the app works fully offline. Each state was given time to stabilize before reading smaps (5 seconds for idle, 10 seconds after opening the editor). Process identification was done with ps aux filtering for euro-office and WebKit process names. Results were cross-verified with smem.

## Raw smem output

Idle:

```
Command                         Swap      USS      PSS      RSS
WebKitNetworkProcess               0     9.9M    17.8M    52.4M
euro-office-lite                   0    56.9M    88.5M   169.6M
WebKitWebProcess                   0    78.4M   107.6M   185.0M
                                   0   145.2M   213.9M   407.0M
```

Editing:

```
Command                         Swap      USS      PSS      RSS
WebKitNetworkProcess               0     9.7M    17.7M    52.4M
euro-office-lite                   0   140.2M   172.1M   253.4M
WebKitWebProcess                   0   455.3M   485.4M   564.2M
                                   0   605.2M   692.4M   892.3M
```

---

## Re-measurement with the production frontend (v0.17.0-alpha, 2026-07-23)

Same machine (bare-metal Ubuntu 22.04.5, kernel 6.8.0-124-generic, 16 GB RAM), same protocol: PSS via smem, states stabilized (start screen idle; blank .docx opened, no typing; second sample at +30 s confirmed flat within 0.3 MB). Fresh install from the official v0.17.0-alpha release .deb after a full purge (package, user data, caches).

### Idle (start screen, no document)

| Process | USS | PSS | RSS |
|---|---|---|---|
| euro-office-lite (Tauri/Rust) | 49.4 MB | 73.8 MB | 153.6 MB |
| WebKitWebProcess | 62.5 MB | 82.2 MB | 156.5 MB |
| WebKitNetworkProcess | 9.9 MB | 17.8 MB | 52.7 MB |
| **Total** | **121.8 MB** | **173.8 MB** | **362.8 MB** |

### Editing (blank .docx, writer editor loaded)

| Process | USS | PSS | RSS |
|---|---|---|---|
| euro-office-lite (Tauri/Rust) | 65.0 MB | 89.9 MB | 170.3 MB |
| WebKitWebProcess | 355.8 MB | 376.6 MB | 452.5 MB |
| WebKitNetworkProcess | 9.9 MB | 18.1 MB | 53.5 MB |
| **Total** | **430.7 MB** | **484.7 MB** | **676.2 MB** |

### Deltas

| State | Dev build (2026-06-28) | v0.17.0 production | Change |
|---|---|---|---|
| Idle PSS | 213.9 MB | 173.8 MB | **-40 MB (-19%)** |
| Editing PSS | 675.4 MB | 484.7 MB | **-191 MB (-28%)** |

Against Euro-Office Desktop measured on this same machine on 2026-07-02 (413 MB idle / 564 MB editing, PSS, all child processes): v0.17.0 is now lighter in BOTH states, -58% idle and -14% editing. Caveat for public claims: their number is from 2026-07-02 with their version of that date; re-install and re-measure theirs the same day before publishing a head-to-head comparison.

Note: the editing document was opened by passing the bundled blank.docx template as a CLI argument, which follows the same open pipeline as the start-screen Document button (verified via [OPEN] log entries).
