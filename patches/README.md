# Submodule patches

Changes carried on top of the `src/web-apps` and `src/sdkjs` submodules until
the submodule revision includes them. `scripts/apply-patches.mjs` applies each
folder as one series, in file-name order:

- `build-frontend-prod.mjs` (release frontend) and `prepare-dist.sh` /
  `prepare-dist.ps1` (development builds) apply them before building or copying
  and take them back afterwards, so the submodules end as they started.
- `compile-doctrenderer-sdkjs.sh` applies `doctrenderer-sdkjs/` to the sdkjs tag
  it checks out for x2t.
- For `tauri dev`, apply them once by hand and take them back before updating a
  submodule:

```
node scripts/apply-patches.mjs apply
node scripts/apply-patches.mjs status
node scripts/apply-patches.mjs revert
```

A build stops when a series no longer applies. After a submodule update that
usually means upstream already has the change: remove that patch from here.

## Origin

| Patch | Origin |
| --- | --- |
