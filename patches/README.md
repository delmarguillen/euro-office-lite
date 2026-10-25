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
| `web-apps/0001-onlyoffice-61a71b365.patch` | ONLYOFFICE/web-apps `61a71b3659a1a35a641fc9ffe3c7c0df8b3e6878` |
| `web-apps/0002-onlyoffice-6c82f398f.patch` | ONLYOFFICE/web-apps `6c82f398fcc92c70591177011ab538d8b0695484` |
| `web-apps/0003-onlyoffice-caf566bf3.patch` | ONLYOFFICE/web-apps `caf566bf37180d9d947a5d773f651b5b26609feb` |
| `web-apps/0004-onlyoffice-4bb7a73f6.patch` | ONLYOFFICE/web-apps `4bb7a73f652a83f6424fc77caa957f3a07ae108a` |
| `web-apps/0005-onlyoffice-c34647af7.patch` | ONLYOFFICE/web-apps `c34647af7fcf92e90deab92d59993c2d3ca61e1a` (applies after 0004) |
