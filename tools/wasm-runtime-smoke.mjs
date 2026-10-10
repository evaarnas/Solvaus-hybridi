#!/usr/bin/env node
// Executes the generated browser WASM in Node. A wasm32 compile alone cannot catch
// runtime-only traps such as std::time::Instant on wasm32-unknown-unknown.
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const glueUrl = new URL('../web/pkg/cube_wasm.js', import.meta.url);
const wasmUrl = new URL('../web/pkg/cube_wasm_bg.wasm', import.meta.url);
const { default: init, CubeLab } = await import(glueUrl.href);
await init({ module_or_path: await readFile(wasmUrl) });

function applyWideTurn(cube, n, axis, start, width, dir) {
  for (let layer = start; layer < start + width; layer += 1) {
    cube.apply_design_move(axis, layer, dir);
  }
}

function smokeReduction(n, setup) {
  const source = new CubeLab(n);
  setup(source);
  const colors = source.face_colors(n);
  assert.equal(colors.length, 6 * n * n, 'complete sticker buffer');
  const solver = new CubeLab(n);
  assert.equal(solver.load_face_colors(colors), true, 'sticker-only worker boundary');
  assert.equal(solver.is_solved(), false, 'smoke state must be scrambled');

  const started = performance.now();
  const result = JSON.parse(solver.solve(6, 6_000));
  assert.equal(result.found, true, `WASM ${n}x${n} reduction failed: ${JSON.stringify(result)}`);
  assert.equal(result.winner, 'reduction');

  const replay = new CubeLab(n);
  assert.equal(replay.load_face_colors(colors), true);
  for (const move of result.moves) {
    replay.apply_design_move(move.axis, move.layer, move.dir);
  }
  assert.equal(replay.is_solved(), true, 'returned WASM moves must independently replay');

  source.free();
  solver.free();
  replay.free();
  console.log(
    `wasm runtime smoke passed: ${n}x${n}, ${result.moveCount} HTM, ${Math.round(performance.now() - started)} ms`,
  );
}

smokeReduction(4, (cube) => {
  cube.apply_design_move(0, 1, 1);
  cube.apply_design_move(1, 3, -1);
  cube.apply_design_move(2, 0, 1);
});
smokeReduction(11, (cube) => {
  applyWideTurn(cube, 11, 0, 0, 5, 1);
  applyWideTurn(cube, 11, 1, 6, 5, -1);
  applyWideTurn(cube, 11, 2, 0, 5, 1);
  applyWideTurn(cube, 11, 0, 6, 5, -1);
  applyWideTurn(cube, 11, 1, 0, 5, 1);
  applyWideTurn(cube, 11, 2, 6, 5, -1);
});

smokeReduction(21, (cube) => {
  applyWideTurn(cube, 21, 0, 0, 10, 1);
  applyWideTurn(cube, 21, 1, 11, 10, -1);
  applyWideTurn(cube, 21, 2, 0, 10, 1);
  applyWideTurn(cube, 21, 0, 11, 10, -1);
  applyWideTurn(cube, 21, 1, 0, 10, 1);
  applyWideTurn(cube, 21, 2, 11, 10, -1);
});
