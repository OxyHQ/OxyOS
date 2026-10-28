# Solar Gradients

Imported from the HEIC supplied by the user, using `scripts/import-solar-wallpaper.swift`. The original file remains at its supplied location; its SHA-256 and byte count are recorded in `manifest.json`. No ownership or relicensing of the supplied artwork is asserted by this import.

- `frames/frame-00.jpg` through `frame-15.jpg`: all 16 frames at the original 5120 × 2880 resolution, JPEG quality 0.96, embedded Display P3 profile. These are converted derivatives, not byte-identical HEIC payloads.
- `manifest.json`: resource identity, source provenance, dimensions and solar anchors.
- `solar-original.plist` and `solar-original.json`: the original decoded Apple solar metadata, preserved without rewriting its values.

The `a`, `z`, `i` values in the source metadata are represented as elevation, azimuth and frame index in the manifest. The sequence spans night, morning twilight, sunrise, day, sunset and evening twilight. These are **solar positions, not fixed clock times**.

The browser host currently supplies frame 08 to the shared shell wallpaper component. There is no recreated Settings application. `apps/shell/src/solar-wallpaper.ts` can select an original frame from a supplied solar elevation and rising/setting phase. It does not compute solar position or request location. A real system wallpaper service must supply the position using the chosen location/time policy, own wallpaper preferences, react to clock/timezone/resume changes, and publish its chosen image to the shell. That service is not implemented here. There are no invented hourly thresholds or background location requests.

The files are repository resources, not yet installed into the Debian image. The image packaging and native image-decoder integration remain part of the real session work. Bloom receives an image source and renders it; time/location policy belongs to OxyOS.

To re-import on macOS:

```sh
swift scripts/import-solar-wallpaper.swift '/path/to/Solar Gradients.heic' assets/wallpapers/solar-gradients
```

The import utility is a developer tool. Swift, ImageIO and CryptoKit are not target runtime dependencies.
