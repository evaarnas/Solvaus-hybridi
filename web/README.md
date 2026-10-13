# Cube Solver web interface

The polished Studio/Swarm UI, generated from `tools/design-source.txt` and wired to the real Rust solver by `tools/gen-index.py`. Mostly based on [Cube-Solver](https://github.com/AdamNolle/Cube-Solver) by Adam Nolle and on [RCube](https://github.com/ShellPuppy/RCube), bundled as `src-tauri/rcube`.

## Supported sizes

- **Standalone browser:** 2×2–21×21
- **Tauri desktop:** 2×2–1001×1001

Browser 9×9–21×21 reductions have a 570-second solver deadline and ten-minute UI watchdog; desktop solves have a five-minute watchdog and a 1001×1001 maximum. Cubes larger than 21×21 use the texture-based renderer, which samples up to 512 stickers per face and adapts redraw frequency for large faces. The display is downsampled above that resolution; solver state still uses every sticker. Browser and native cubes through 21×21 replay returned moves through the cube model. Native texture-rendered cubes above 21×21 use RCube's solved-state result directly. Solves can be cancelled. Solve time and move count depend on the state and device.

## Run in a browser

Build the generated WASM package:

```sh
wasm-pack build crates/cube_wasm --release --target web \
  --out-dir "$PWD/web/pkg" --out-name cube_wasm
```

Serve the directory over HTTP:

```sh
python3 -m http.server -d web 8000
```

Open <http://localhost:8000>. ES modules and WebAssembly do not work through a direct `file://` URL.

## Solver routing

- 2×2/3×3 and standalone-browser 4×4–21×21 run in `solver-worker.js`.
- Tauri desktop 4×4–1001×1001 uses native `solve_stickers` / `cancel_solve` commands.
- Tauri desktop 4×4–1001×1001 generates scrambles with RCube's native `Cube::Scramble` routine; the frontend receives only replayable legal turns.
- Studio solve requests contain the complete sticker-color state—not scramble history.
- Browser and native cubes through 21×21 replay returned moves through the cube model; native texture-rendered cubes above 21×21 use RCube's solved-state result directly.

Custom scrambles can be entered with `U D L R F B` notation, prime/half-turn suffixes, and supported wide turns such as `Rw` or `3Rw2`. The interactive controls use the same legal move representation.

## Generated source

Do not edit `web/index.html` directly.

1. Edit `tools/design-source.txt` for authored UI changes.
2. Edit `tools/gen-index.py` for runtime/solver wiring.
3. Regenerate and verify:

```sh
python3 tools/gen-index.py
python3 tools/frontend-smoke.py
```

three.js is vendored under `web/vendor/`; the packaged interface has no runtime CDN dependency.
