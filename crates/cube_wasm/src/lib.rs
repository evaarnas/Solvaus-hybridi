//! WebAssembly bridge exposing the real `cube_core` model and `cube_solver`
//! engines to the web UI. Single-threaded (the `parallel` feature of
//! `cube_solver` is disabled), so the solver race runs sequentially in the
//! browser — same results, just serial.

use cube_core::{Axis, Color, CubeSize, CubeState, Face, Move, StickerCube};
use cube_solver::{run_solver_lab_tiered, wide_move_set, SolverBudget};
use std::time::Duration;
use wasm_bindgen::prelude::*;

/// Stable 0..6 index for a color (matches the JS palette order).
fn color_index(c: Color) -> u8 {
    match c {
        Color::White => 0,
        Color::Yellow => 1,
        Color::Green => 2,
        Color::Blue => 3,
        Color::Orange => 4,
        Color::Red => 5,
    }
}

fn color_from_index(index: u8) -> Option<Color> {
    Some(match index {
        0 => Color::White,
        1 => Color::Yellow,
        2 => Color::Green,
        3 => Color::Blue,
        4 => Color::Orange,
        5 => Color::Red,
        _ => return None,
    })
}

fn axis_index(a: Axis) -> u8 {
    match a {
        Axis::X => 0,
        Axis::Y => 1,
        Axis::Z => 2,
    }
}

thread_local! {
    /// The two-phase 3×3 solver builds a few MB of lookup tables once; cache it on
    /// the (single) worker thread so only the first solve pays the build cost.
    static KOCIEMBA: std::cell::OnceCell<cube_solver::kociemba::search::Solver> =
        const { std::cell::OnceCell::new() };
}

fn with_kociemba<R>(f: impl FnOnce(&cube_solver::kociemba::search::Solver) -> R) -> R {
    KOCIEMBA.with(|c| f(c.get_or_init(cube_solver::kociemba::search::Solver::new)))
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen(
    inline_js = "export function post_solve_progress(percent) { if (typeof self !== 'undefined' && typeof self.postMessage === 'function') { self.postMessage({ type: 'progress', percent: percent }); } }"
)]
extern "C" {
    /// Streams the stage-based estimate to the main thread from inside the worker.
    fn post_solve_progress(percent: f64);
}

/// Build the 3×3 two-phase tables ahead of the first solve. The worker calls this
/// once at startup so the first real Solve isn't slowed by the ~one-time table build.
#[wasm_bindgen]
pub fn warm_solver() {
    with_kociemba(|_| {});
}

fn reduction_time_limit(n: usize) -> Duration {
    let seconds: u64 = if n <= 5 {
        28
    } else if n <= 8 {
        115
    } else {
        290
    };
    // Debug test binaries are substantially slower on hosted Windows and run
    // several solver tests concurrently. Keep the product watchdog unchanged,
    // but give correctness tests a finite margin so scheduler contention cannot
    // masquerade as a solver failure.
    #[cfg(test)]
    let seconds = seconds.saturating_mul(4);
    Duration::from_secs(seconds)
}

fn wasm_reduction_time_limit(n: usize) -> Duration {
    if n <= 8 {
        reduction_time_limit(n)
    } else {
        // Leave time for replay verification and result serialization under the
        // browser's ten-minute solve watchdog.
        Duration::from_secs(570)
    }
}

/// A tiny deterministic RNG (xorshift64*) so scrambles are reproducible by seed
/// without pulling OS entropy.
struct Rng(u64);
impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed | 1)
    }
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    /// Uniform integer in `0..n` via Lemire's debiased multiply-shift (no modulo
    /// bias). `n` is always >= 1 at every call site.
    fn below(&mut self, n: usize) -> usize {
        ((self.next() as u128 * n as u128) >> 64) as usize
    }
}

#[wasm_bindgen]
pub struct CubeLab {
    cube: StickerCube,
    n: usize,
    solution: Vec<Move>,
}

#[wasm_bindgen]
impl CubeLab {
    #[wasm_bindgen(constructor)]
    pub fn new(n: usize) -> CubeLab {
        #[cfg(target_arch = "wasm32")]
        console_error_panic_hook::set_once();
        let n = n.clamp(2, 2000);
        let size = CubeSize::new(n).expect("valid size");
        CubeLab {
            cube: StickerCube::solved(size),
            n,
            solution: Vec::new(),
        }
    }

    pub fn set_size(&mut self, n: usize) {
        let n = n.clamp(2, 2000);
        if n != self.n {
            self.n = n;
            self.cube = StickerCube::solved(CubeSize::new(n).expect("valid size"));
            self.solution.clear();
        }
    }

    pub fn size(&self) -> usize {
        self.n
    }

    pub fn reset(&mut self) {
        self.cube = StickerCube::solved(CubeSize::new(self.n).expect("valid size"));
        self.solution.clear();
    }

    pub fn is_solved(&self) -> bool {
        self.cube.is_solved()
    }

    pub fn solved_percent(&self) -> f64 {
        let total = (6 * self.n * self.n) as f64;
        let mism = self.cube.mismatch_count() as f64;
        (100.0 * (1.0 - mism / total)).clamp(0.0, 100.0)
    }

    /// Visible (surface) pieces: N³ minus the hidden inner core.
    pub fn piece_count(&self) -> usize {
        let n = self.n;
        if n < 2 {
            0
        } else {
            n * n * n - (n - 2) * (n - 2) * (n - 2)
        }
    }

    /// Span of wide moves used for this cube (1 for the 3×3, wider for big cubes).
    fn scramble_span(&self) -> usize {
        if self.n <= 3 {
            1
        } else {
            3.min(self.n - 1)
        }
    }

    /// Scramble from solved with `depth` random moves; returns the count applied.
    /// Reproducible for a given `seed`.
    pub fn scramble(&mut self, depth: usize, seed: u64) -> usize {
        let size = CubeSize::new(self.n).expect("valid size");
        self.cube = StickerCube::solved(size);
        self.solution.clear();
        let moves = wide_move_set(size, self.scramble_span());
        if moves.is_empty() {
            return 0;
        }
        let mut rng = Rng::new(seed ^ 0x9E37_79B9_7F4A_7C15);
        let mut applied = 0usize;
        let mut last: Option<Move> = None;
        let mut guard = 0usize;
        while applied < depth && guard < depth * 20 + 50 {
            guard += 1;
            let mv = moves[rng.below(moves.len())];
            if last
                .map(|p| p.axis == mv.axis && p.layer_start == mv.layer_start)
                .unwrap_or(false)
            {
                continue;
            }
            if self.cube.apply_move(mv).is_ok() {
                applied += 1;
                last = Some(mv);
            }
        }
        applied
    }

    /// Flat color indices (0..6) for rendering: 6 faces × dim × dim, in
    /// `Face::ALL` order (Up, Down, Front, Back, Left, Right), row-major. Faces
    /// larger than `sample` are down-sampled.
    pub fn face_colors(&self, sample: usize) -> Vec<u8> {
        let dim = self.render_dim(sample);
        let mut out = Vec::with_capacity(6 * dim * dim);
        for face in Face::ALL {
            let fs = self.cube.face_sample(face, dim);
            for row in &fs.cells {
                for c in row {
                    out.push(color_index(*c));
                }
            }
        }
        out
    }

    pub fn render_dim(&self, sample: usize) -> usize {
        sample.min(self.n).max(1)
    }

    /// Replace the model from a complete sticker-state buffer in `Face::ALL`
    /// order. The solver worker uses this boundary so it receives no scramble
    /// history to invert—only the same colors visible to the user.
    pub fn load_face_colors(&mut self, colors: &[u8]) -> bool {
        if colors.len() != 6 * self.n * self.n {
            return false;
        }
        let Some(stickers) = colors
            .iter()
            .copied()
            .map(color_from_index)
            .collect::<Option<Vec<_>>>()
        else {
            return false;
        };
        let Ok(snapshot) = serde_json::from_value::<cube_core::CubeSnapshot>(
            serde_json::json!({ "size": self.n, "stickers": stickers }),
        ) else {
            return false;
        };
        let cube = StickerCube::from_snapshot(snapshot);
        if cube.validate().is_err() {
            return false;
        }
        self.cube = cube;
        self.solution.clear();
        true
    }

    /// Apply one quarter turn in the web UI's `{axis, layer, dir}` convention,
    /// which is identical to `cube_core`'s (axes X/Y/Z = 0/1/2, layer index along
    /// the axis, dir ±1 = right-hand quarter turn). Used to mirror the on-screen
    /// scramble into the solver's cube so the returned solution matches exactly.
    pub fn apply_design_move(&mut self, axis: u8, layer: usize, dir: i32) {
        if layer >= self.n {
            return; // ignore out-of-range layers (e.g. a stale move after N shrank)
        }
        let ax = match axis {
            0 => Axis::X,
            1 => Axis::Y,
            _ => Axis::Z,
        };
        let turns: i8 = if dir >= 0 { 1 } else { -1 };
        let mv = Move::new(ax, layer, layer, turns);
        let _ = self.cube.apply_move(mv);
    }

    /// Solve a 3×3 with the two-phase (Kociemba) solver. Returns the same JSON shape
    /// as `solve`, or `None` if conversion/solve fails (then `solve` falls back).
    fn try_kociemba(&mut self) -> Option<String> {
        let solution =
            with_kociemba(|s| cube_solver::kociemba::cube3::solve_sticker(&self.cube, s))?;
        let size = cube_core::CubeSize::new(3).ok()?;
        let mut moves_json: Vec<serde_json::Value> = Vec::new();
        let mut notation: Vec<String> = Vec::new();
        for m in &solution {
            notation.push(m.notation(size));
            let axis = axis_index(m.axis);
            let (count, dir): (usize, i32) = match m.turns.rem_euclid(4) {
                1 => (1, 1),
                2 => (2, 1),
                3 => (1, -1),
                _ => (0, 1),
            };
            for layer in m.layer_start..=m.layer_end {
                for _ in 0..count {
                    moves_json
                        .push(serde_json::json!({ "axis": axis, "layer": layer, "dir": dir }));
                }
            }
        }
        self.solution = solution;
        // Report the standard face-turn (HTM) count — one per notation entry, where a
        // half-turn like U2 is a single move. `moves_json` stays half-turn-expanded
        // (U2 -> two quarter steps) purely so the animation plays each quarter; using
        // its length as the move count would inflate the figure (~34 vs ~22) and
        // disagree with both the README and the 2×2 path (which report HTM moves).
        let htm = notation.len();
        Some(
            serde_json::json!({
                "found": true,
                "winner": "kociemba",
                "moveCount": htm,
                "elapsedMs": 0,
                "moves": moves_json,
                "notation": notation,
                "lanes": [ { "id": "kociemba", "pct": 100, "moveCount": htm, "label": "two-phase", "solved": true } ],
            })
            .to_string(),
        )
    }

    /// Solve a 4×4+ cube with the NxN reduction method (centres → edges → 3×3 finish →
    /// parity). Same JSON shape as `try_kociemba`, or `None` if the reduction fails (then
    /// `solve` falls back to the legacy engine race). Solves a CLONE — `solve_reduction`
    /// mutates its cube to solved — so `self.cube` stays scrambled for the animation.
    fn try_reduction(&mut self) -> Option<String> {
        // Finish before the browser's worker watchdog so timeout can unwind through
        // reduction loops cleanly; worker termination remains the hard-stop backstop.
        let control = cube_solver::reduction::ReductionControl::with_timeout(
            wasm_reduction_time_limit(self.n),
        );
        #[cfg(target_arch = "wasm32")]
        {
            return cube_solver::reduction::with_progress_observer(
                |percent| post_solve_progress(percent),
                || self.try_reduction_with_control(&control),
            );
        }
        #[cfg(not(target_arch = "wasm32"))]
        self.try_reduction_with_control(&control)
    }

    fn try_reduction_with_control(
        &mut self,
        control: &cube_solver::reduction::ReductionControl,
    ) -> Option<String> {
        let original = self.cube.clone();
        let mut work = original.clone();
        let solution = with_kociemba(|s| {
            cube_solver::reduction::solve_reduction_with_control(&mut work, s, control)
        })
        .ok()?;
        if !work.is_solved() {
            return None;
        }
        let mut replay = original;
        for &mv in &solution {
            replay.apply_move(mv).ok()?;
        }
        if !replay.is_solved() {
            return None;
        }
        let size = CubeSize::new(self.n).ok()?;
        let mut moves_json: Vec<serde_json::Value> = Vec::new();
        let mut notation: Vec<String> = Vec::new();
        for m in &solution {
            notation.push(m.notation(size));
            let axis = axis_index(m.axis);
            let (count, dir): (usize, i32) = match m.turns.rem_euclid(4) {
                1 => (1, 1),
                2 => (2, 1),
                3 => (1, -1),
                _ => (0, 1),
            };
            for layer in m.layer_start..=m.layer_end {
                for _ in 0..count {
                    moves_json
                        .push(serde_json::json!({ "axis": axis, "layer": layer, "dir": dir }));
                }
            }
        }
        self.solution = solution;
        let htm = notation.len();
        Some(
            serde_json::json!({
                "found": true,
                "winner": "reduction",
                "moveCount": htm,
                "elapsedMs": 0,
                "moves": moves_json,
                "notation": notation,
                "lanes": [ { "id": "reduction", "pct": 100, "moveCount": htm, "label": "reduction", "solved": true } ],
            })
            .to_string(),
        )
    }

    /// Run the solver on the current state and store the winning (fewest-move,
    /// replay-verified) solution. 3×3 uses the two-phase solver; 4×4+ (up to
    /// `MAX_REDUCTION`) uses the reduction method; other sizes use the legacy engine
    /// race. Returns a JSON summary.
    pub fn solve(&mut self, max_depth: usize, time_ms: f64) -> String {
        let snapshot = self.cube.clone_snapshot();
        // 3×3: the two-phase (Kociemba) solver returns a verified solution for ANY
        // scramble — not just the <=9-move ones the legacy search can reach. Fall
        // through to the legacy search only if it somehow fails.
        if self.n == 3 {
            if let Some(json) = self.try_kociemba() {
                return json;
            }
        }
        // 4×4 and up: the reduction method returns a verified real solution. The
        // browser supports up to 21×21 with a size-aware deadline and worker watchdog.
        const MAX_REDUCTION: usize = 21;
        if self.n > 3 && self.n <= MAX_REDUCTION {
            if let Some(json) = self.try_reduction() {
                return json;
            }
        }
        let depth = max_depth.clamp(1, 9);
        let mut budget = SolverBudget::for_depth(depth);
        budget.time_limit = Duration::from_millis(time_ms.max(50.0) as u64);
        // The web UI scrambles with outer-face turns only, so the solver searches
        // the same (outer) move set — guaranteeing it can invert the scramble.
        budget.max_wide = 1;
        // Scale the search budget with depth so deeper scrambles still solve, but
        // cap it so a hard scramble can never freeze the page for more than a
        // moment (the meet-in-the-middle frontier grows ~18^(depth/2)).
        let half = depth.div_ceil(2) as u32;
        budget.max_nodes = 18usize
            .saturating_pow(half)
            .saturating_mul(2)
            .clamp(400_000, 1_200_000);

        // The exact solver gets the full budget; the heuristic engines get a small
        // slice (just for the race display), so a deep solve never runs all three
        // at full budget.
        let mut secondary = budget;
        secondary.time_limit = Duration::from_millis(280);
        secondary.max_nodes = 60_000;
        let run = run_solver_lab_tiered(snapshot, budget, secondary);

        // Per-worker latest state for the race lanes.
        let lane_ids = ["deterministic", "beam", "evolution"];
        let mut lanes = Vec::new();
        for id in lane_ids {
            let last = run.events.iter().rev().find(|e| e.worker_id == id);
            let solved = run.events.iter().any(|e| {
                e.worker_id == id && e.candidate.as_ref().map(|c| c.solved).unwrap_or(false)
            });
            let (pct, mc, label) = match last {
                Some(e) => (
                    (e.best_fitness.clamp(0.0, 1.0) * 100.0) as i32,
                    if e.best_move_count == usize::MAX {
                        -1
                    } else {
                        e.best_move_count as i32
                    },
                    e.message.clone(),
                ),
                None => (0, -1, "idle".to_string()),
            };
            lanes.push(serde_json::json!({
                "id": id, "pct": pct, "moveCount": mc, "label": label, "solved": solved
            }));
        }

        match run.best {
            Some(best) => {
                self.solution = best.moves.clone();
                // Decompose into single-layer quarter turns {axis, layer, dir} —
                // the format the on-screen cube animates. Parallel layers of a
                // wide turn commute, so splitting is exact.
                let size = CubeSize::new(self.n).unwrap();
                let mut moves: Vec<serde_json::Value> = Vec::new();
                let mut notation: Vec<String> = Vec::new();
                for m in &best.moves {
                    notation.push(m.notation(size));
                    let axis = axis_index(m.axis);
                    let t = m.turns.rem_euclid(4);
                    let (count, dir) = match t {
                        1 => (1, 1),
                        2 => (2, 1),
                        3 => (1, -1),
                        _ => (0, 1),
                    };
                    for layer in m.layer_start..=m.layer_end {
                        for _ in 0..count {
                            moves.push(serde_json::json!({
                                "axis": axis, "layer": layer, "dir": dir
                            }));
                        }
                    }
                }
                serde_json::json!({
                    "found": true,
                    "winner": best.worker_id,
                    "moveCount": best.move_count,
                    "elapsedMs": best.elapsed_ms,
                    "moves": moves,
                    "notation": notation,
                    "lanes": lanes,
                })
                .to_string()
            }
            None => {
                self.solution.clear();
                serde_json::json!({ "found": false, "lanes": lanes }).to_string()
            }
        }
    }

    pub fn solution_len(&self) -> usize {
        self.solution.len()
    }

    /// Apply the `i`-th move of the stored solution to the live cube.
    pub fn apply_solution_step(&mut self, i: usize) -> bool {
        if i < self.solution.len() {
            self.cube.apply_move(self.solution[i]).is_ok()
        } else {
            false
        }
    }
}

/// Native desktop entry point for the same sticker-only reduction boundary used by
/// the Web Worker. Tauri owns the control handle so Cancel can cooperatively stop
/// CPU work instead of merely ignoring a stale result.
#[cfg(not(target_arch = "wasm32"))]
pub use cube_solver::reduction::ReductionControl as NativeReductionControl;

#[cfg(not(target_arch = "wasm32"))]
pub fn native_reduction_control(n: usize) -> NativeReductionControl {
    NativeReductionControl::with_timeout(reduction_time_limit(n))
}

#[cfg(not(target_arch = "wasm32"))]
pub fn solve_reduction_sticker_state(
    n: usize,
    colors: &[u8],
    control: &NativeReductionControl,
) -> Result<String, String> {
    if !(4..=11).contains(&n) {
        return Err("native reduction supports 4x4 through 11x11".to_string());
    }
    let mut lab = CubeLab::new(n);
    if !lab.load_face_colors(colors) {
        return Err("invalid or incomplete sticker state".to_string());
    }
    match lab.try_reduction_with_control(control) {
        Some(result) => Ok(result),
        None if !control.should_continue() => Err("solve cancelled or timed out".to_string()),
        None => Err("no replay-verified solution was found".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_reduction_budget_is_ten_minutes_without_extending_native_budget() {
        let test_margin = if cfg!(test) { 4 } else { 1 };
        assert_eq!(
            wasm_reduction_time_limit(8),
            Duration::from_secs(115 * test_margin)
        );
        assert_eq!(wasm_reduction_time_limit(21), Duration::from_secs(570));
        assert_eq!(
            reduction_time_limit(21),
            Duration::from_secs(290 * test_margin)
        );
    }

    #[test]
    fn sticker_state_boundary_round_trips_without_scramble_history() {
        let mut source = CubeLab::new(3);
        source.apply_design_move(0, 2, 1);
        source.apply_design_move(1, 0, -1);
        source.apply_design_move(2, 2, 1);
        let colors = source.face_colors(3);

        let mut worker = CubeLab::new(3);
        assert!(worker.load_face_colors(&colors));
        assert_eq!(worker.face_colors(3), colors);
        assert!(!worker.is_solved());

        let mut invalid = colors.clone();
        invalid[0] = 9;
        assert!(!worker.load_face_colors(&invalid));
        assert!(!worker.load_face_colors(&colors[..colors.len() - 1]));
    }

    /// Exactly what the web UI does: load only the scrambled sticker state, run a
    /// real solve, then apply the returned `{axis,layer,dir}` moves — must solve.
    #[test]
    fn outer_scramble_is_really_solved() {
        for n in [2usize, 3, 4, 5] {
            let mut lab = CubeLab::new(n);
            let mut rng = Rng::new(1234 + n as u64);
            for _ in 0..6 {
                let axis = rng.below(3) as u8;
                let layer = if rng.below(2) == 0 { 0 } else { n - 1 };
                let dir = if rng.below(2) == 0 { 1 } else { -1 };
                lab.apply_design_move(axis, layer, dir);
            }
            assert!(!lab.is_solved(), "n={n} should be scrambled");

            let json = lab.solve(8, 3000.0);
            let v: serde_json::Value = serde_json::from_str(&json).unwrap();
            assert!(v["found"].as_bool().unwrap(), "no solution found for n={n}");

            for m in v["moves"].as_array().unwrap() {
                lab.apply_design_move(
                    m["axis"].as_u64().unwrap() as u8,
                    m["layer"].as_u64().unwrap() as usize,
                    m["dir"].as_i64().unwrap() as i32,
                );
            }
            assert!(lab.is_solved(), "returned solution did not solve n={n}");
        }
    }

    /// 4×4–6×6 route through the REAL reduction solver (winner == "reduction", not the
    /// legacy/visual fallback) under deep inner+outer scrambles, and the returned moves
    /// genuinely solve the cube — the end-to-end wiring of `try_reduction`.
    #[test]
    fn reduction_path_solves() {
        for n in [4usize, 5] {
            let mut lab = CubeLab::new(n);
            let mut rng = Rng::new(99 + n as u64);
            for _ in 0..(n * 6) {
                let axis = rng.below(3) as u8;
                let layer = rng.below(n); // inner slices too, not just outer
                let dir = if rng.below(2) == 0 { 1 } else { -1 };
                lab.apply_design_move(axis, layer, dir);
            }
            assert!(!lab.is_solved(), "n={n} should be scrambled");
            let v: serde_json::Value = serde_json::from_str(&lab.solve(8, 3000.0)).unwrap();
            assert!(v["found"].as_bool().unwrap(), "n={n} no solution");
            assert_eq!(
                v["winner"], "reduction",
                "n={n} did not use the reduction solver"
            );
            for m in v["moves"].as_array().unwrap() {
                lab.apply_design_move(
                    m["axis"].as_u64().unwrap() as u8,
                    m["layer"].as_u64().unwrap() as usize,
                    m["dir"].as_i64().unwrap() as i32,
                );
            }
            assert!(lab.is_solved(), "n={n} reduction solution did not solve");
        }
    }

    #[test]
    fn native_sticker_entrypoint_solves_and_replays() {
        let n = 4;
        let mut source = CubeLab::new(n);
        for (axis, layer, dir) in [(0, 1, 1), (1, 3, -1), (2, 0, 1)] {
            source.apply_design_move(axis, layer, dir);
        }
        let colors = source.face_colors(n);
        let control = native_reduction_control(n);
        let json = solve_reduction_sticker_state(n, &colors, &control).unwrap();
        let result: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(result["winner"], "reduction");

        let mut replay = CubeLab::new(n);
        assert!(replay.load_face_colors(&colors));
        for mv in result["moves"].as_array().unwrap() {
            replay.apply_design_move(
                mv["axis"].as_u64().unwrap() as u8,
                mv["layer"].as_u64().unwrap() as usize,
                mv["dir"].as_i64().unwrap() as i32,
            );
        }
        assert!(replay.is_solved());

        let cancelled = native_reduction_control(n);
        cancelled.cancel();
        assert_eq!(
            solve_reduction_sticker_state(n, &colors, &cancelled),
            Err("solve cancelled or timed out".to_string())
        );
    }

    #[test]
    #[ignore = "desktop 11x11 native reduction release gate; run explicitly"]
    fn native_n11_sticker_entrypoint_solves_and_replays() {
        let n = 11;
        let mut source = CubeLab::new(n);
        for (axis, start, end, dir) in [
            (0, 0, 2, 1),
            (1, 9, 10, -1),
            (2, 0, 3, 1),
            (0, 8, 10, -1),
            (1, 0, 1, 1),
            (2, 9, 10, -1),
        ] {
            for layer in start..=end {
                source.apply_design_move(axis, layer, dir);
            }
        }
        let colors = source.face_colors(n);
        let control = native_reduction_control(n);
        let json = solve_reduction_sticker_state(n, &colors, &control).unwrap();
        let result: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(result["winner"], "reduction");

        let mut replay = CubeLab::new(n);
        assert!(replay.load_face_colors(&colors));
        for mv in result["moves"].as_array().unwrap() {
            replay.apply_design_move(
                mv["axis"].as_u64().unwrap() as u8,
                mv["layer"].as_u64().unwrap() as usize,
                mv["dir"].as_i64().unwrap() as i32,
            );
        }
        assert!(replay.is_solved());
    }

    /// The depth-scaled budget must crack deeper 3×3 scrambles (the "auto-stronger
    /// on deep" behaviour), up to the depth-10 ceiling the UI promises.
    #[test]
    fn deep_scrambles_still_solve() {
        for depth in [8usize, 9] {
            let mut lab = CubeLab::new(3);
            let mut rng = Rng::new(7 + depth as u64);
            let mut last: i64 = -1;
            let mut applied = 0;
            while applied < depth {
                let axis = rng.below(3) as u8;
                let layer = if rng.below(2) == 0 { 0 } else { 2 };
                let key = axis as i64 * 3 + layer as i64;
                if key == last {
                    continue;
                }
                last = key;
                let dir = if rng.below(2) == 0 { 1 } else { -1 };
                lab.apply_design_move(axis, layer, dir);
                applied += 1;
            }
            let v: serde_json::Value = serde_json::from_str(&lab.solve(depth, 5000.0)).unwrap();
            assert!(v["found"].as_bool().unwrap(), "depth {depth} not solved");
            for m in v["moves"].as_array().unwrap() {
                lab.apply_design_move(
                    m["axis"].as_u64().unwrap() as u8,
                    m["layer"].as_u64().unwrap() as usize,
                    m["dir"].as_i64().unwrap() as i32,
                );
            }
            assert!(lab.is_solved(), "depth {depth} solution did not solve");
        }
    }

    /// The two-phase solver cracks ANY 3×3 scramble — far past the legacy ≤9-move
    /// limit — and `solve()` routes every 3×3 through it.
    #[test]
    fn kociemba_solves_arbitrarily_deep_3x3() {
        let mut lab = CubeLab::new(3);
        let mut s: u64 = 0x9E37_79B9_7F4A_7C15;
        for _ in 0..40 {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            let axis = (s % 3) as u8;
            let layer: usize = if (s >> 3) & 1 == 0 { 0 } else { 2 };
            let dir: i32 = if (s >> 4) & 1 == 0 { 1 } else { -1 };
            lab.apply_design_move(axis, layer, dir);
        }
        assert!(
            !lab.is_solved(),
            "a 40-move scramble should not already be solved"
        );
        let v: serde_json::Value = serde_json::from_str(&lab.solve(20, 5000.0)).unwrap();
        assert!(
            v["found"].as_bool().unwrap(),
            "deep 3x3 reported no solution"
        );
        assert_eq!(
            v["winner"].as_str().unwrap(),
            "kociemba",
            "3x3 should use the two-phase solver"
        );
        for m in v["moves"].as_array().unwrap() {
            lab.apply_design_move(
                m["axis"].as_u64().unwrap() as u8,
                m["layer"].as_u64().unwrap() as usize,
                m["dir"].as_i64().unwrap() as i32,
            );
        }
        assert!(
            lab.is_solved(),
            "the two-phase solution did not solve the deep scramble"
        );
    }
}
