// Hide the console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use cube_core::{Axis, CubeSize, Move};
use cube_wasm::NativeReductionControl;
use std::ffi::{c_char, c_void, CStr};
use std::sync::Mutex;
use std::time::Instant;
use tauri::{ipc::Channel, State};

const TEXTURE_RENDER_THRESHOLD: usize = 21;
const MAX_NATIVE_CUBE_SIZE: usize = 1001;

#[repr(C)]
#[derive(Clone, Copy)]
struct RcubeMove {
    face: u8,
    depth: u32,
    turns: i32,
}

type CancellationCheck = unsafe extern "C" fn(*mut c_void) -> bool;
type ProgressObserver = unsafe extern "C" fn(*mut c_void, f64);

struct SolveProgressContext {
    report: Box<dyn Fn(f64) + Send + Sync>,
}

unsafe extern "C" fn rcube_report_progress(context: *mut c_void, percent: f64) {
    let Some(context) = (unsafe { context.cast::<SolveProgressContext>().as_ref() }) else {
        return;
    };
    (context.report)(percent.clamp(0.0, 100.0));
}

extern "C" {
    fn rcube_solve(
        size: u32,
        colors: *const u8,
        color_count: usize,
        is_cancelled: Option<CancellationCheck>,
        cancellation_context: *mut c_void,
        report_progress: Option<ProgressObserver>,
        progress_context: *mut c_void,
        output_moves: *mut *mut RcubeMove,
        output_count: *mut usize,
        error: *mut c_char,
        error_capacity: usize,
    ) -> i32;
    fn rcube_scramble(
        size: u32,
        seed: i32,
        output_moves: *mut *mut RcubeMove,
        output_count: *mut usize,
        error: *mut c_char,
        error_capacity: usize,
    ) -> i32;
    fn rcube_free_moves(moves: *mut RcubeMove);
}

unsafe extern "C" fn rcube_is_cancelled(context: *mut c_void) -> bool {
    let Some(control) = (unsafe { context.cast::<NativeReductionControl>().as_ref() }) else {
        return true;
    };
    !control.should_continue()
}

fn rcube_move_to_core(movement: RcubeMove, n: usize) -> Result<Move, String> {
    let depth = usize::try_from(movement.depth)
        .map_err(|_| "RCube returned an invalid move depth".to_string())?;
    if depth >= n {
        return Err("RCube returned a move outside the cube".to_string());
    }

    let (axis, layer, direction) = match movement.face {
        // RCube and cube_core use opposite positive directions for X and Z.
        0 => (Axis::Z, n - 1 - depth, -1),
        1 => (Axis::X, n - 1 - depth, -1),
        2 => (Axis::Z, depth, 1),
        3 => (Axis::X, depth, 1),
        4 => (Axis::Y, n - 1 - depth, -1),
        5 => (Axis::Y, depth, 1),
        _ => return Err("RCube returned an invalid face".to_string()),
    };
    let turns = movement
        .turns
        .checked_mul(direction)
        .and_then(|turns| i8::try_from(turns).ok())
        .ok_or_else(|| "RCube returned an invalid turn amount".to_string())?;

    Ok(Move::new(axis, layer, layer, turns))
}

fn solve_with_rcube(
    n: usize,
    colors: &[u8],
    control: &NativeReductionControl,
    progress: Option<Box<dyn Fn(f64) + Send + Sync>>,
) -> Result<String, String> {
    if !(4..=MAX_NATIVE_CUBE_SIZE).contains(&n) || colors.len() != 6 * n * n {
        return Err("invalid cube size or sticker buffer".to_string());
    }

    let started = Instant::now();
    let mut raw_moves = std::ptr::null_mut();
    let mut raw_move_count = 0usize;
    let mut error = [0 as c_char; 512];
    let cancellation_context = (control as *const NativeReductionControl)
        .cast_mut()
        .cast::<c_void>();
    let mut progress_context = progress.map(|report| SolveProgressContext { report });
    let progress_observer = progress_context
        .as_mut()
        .map(|_| rcube_report_progress as ProgressObserver);
    let progress_context_ptr = progress_context
        .as_mut()
        .map_or(std::ptr::null_mut(), |context| {
            (context as *mut SolveProgressContext).cast::<c_void>()
        });
    let status = unsafe {
        rcube_solve(
            n as u32,
            colors.as_ptr(),
            colors.len(),
            Some(rcube_is_cancelled),
            cancellation_context,
            progress_observer,
            progress_context_ptr,
            &mut raw_moves,
            &mut raw_move_count,
            error.as_mut_ptr(),
            error.len(),
        )
    };
    if status != 0 {
        let message = unsafe { CStr::from_ptr(error.as_ptr()) }
            .to_string_lossy()
            .into_owned();
        return Err(if message.is_empty() {
            "RCube solve failed".to_string()
        } else {
            message
        });
    }

    if raw_move_count > 0 && raw_moves.is_null() {
        return Err("RCube returned an empty move buffer".to_string());
    }
    let engine_moves = if raw_move_count == 0 {
        Vec::new()
    } else {
        unsafe { std::slice::from_raw_parts(raw_moves, raw_move_count) }.to_vec()
    };
    unsafe { rcube_free_moves(raw_moves) };

    let size = CubeSize::new(n).map_err(|error| error.to_string())?;
    let mut moves = Vec::new();
    let mut notation = Vec::with_capacity(engine_moves.len());
    let mut move_count = 0usize;
    for engine_move in engine_moves {
        let movement = rcube_move_to_core(engine_move, n)?;
        if movement.is_noop() {
            continue;
        }

        move_count += 1;
        if n <= TEXTURE_RENDER_THRESHOLD {
            notation.push(movement.notation(size));
        }
        let axis = match movement.axis {
            Axis::X => 0,
            Axis::Y => 1,
            Axis::Z => 2,
        };
        let (quarter_turns, direction) = match movement.turns.rem_euclid(4) {
            1 => (1, 1),
            2 => (2, 1),
            3 => (1, -1),
            _ => (0, 1),
        };
        for _ in 0..quarter_turns {
            if n <= TEXTURE_RENDER_THRESHOLD {
                moves.push(serde_json::json!({
                    "axis": axis,
                    "layer": movement.layer_start,
                    "dir": direction
                }));
            }
        }
        if n > TEXTURE_RENDER_THRESHOLD {
            let compact_turns = match movement.turns.rem_euclid(4) {
                1 => 1,
                2 => 2,
                3 => -1,
                _ => 0,
            };
            moves.push(serde_json::json!({
                "axis": axis,
                "layer": movement.layer_start,
                "turns": compact_turns
            }));
        }
    }

    Ok(serde_json::json!({
        "found": true,
        "solvedByRcube": true,
        "compactMoves": n > TEXTURE_RENDER_THRESHOLD,
        "winner": "rcube",
        "moveCount": move_count,
        "elapsedMs": started.elapsed().as_millis(),
        "moves": moves,
        "notation": notation,
        "lanes": [{
            "id": "rcube",
            "pct": 100,
            "moveCount": move_count,
            "label": "RCube center and edge reduction",
            "solved": true
        }]
    })
    .to_string())
}

fn scramble_with_rcube(n: usize, seed: i32) -> Result<String, String> {
    if !(4..=MAX_NATIVE_CUBE_SIZE).contains(&n) {
        return Err("invalid cube size".to_string());
    }

    let mut raw_moves = std::ptr::null_mut();
    let mut raw_move_count = 0usize;
    let mut error = [0 as c_char; 512];
    let status = unsafe {
        rcube_scramble(
            n as u32,
            seed,
            &mut raw_moves,
            &mut raw_move_count,
            error.as_mut_ptr(),
            error.len(),
        )
    };
    if status != 0 {
        let message = unsafe { CStr::from_ptr(error.as_ptr()) }
            .to_string_lossy()
            .into_owned();
        return Err(if message.is_empty() {
            "RCube scramble failed".to_string()
        } else {
            message
        });
    }
    if raw_move_count > 0 && raw_moves.is_null() {
        return Err("RCube returned an empty scramble buffer".to_string());
    }

    let engine_moves = if raw_move_count == 0 {
        Vec::new()
    } else {
        unsafe { std::slice::from_raw_parts(raw_moves, raw_move_count) }.to_vec()
    };
    unsafe { rcube_free_moves(raw_moves) };

    let mut moves = Vec::new();
    for engine_move in engine_moves {
        let movement = rcube_move_to_core(engine_move, n)?;
        if movement.is_noop() {
            continue;
        }
        let axis = match movement.axis {
            Axis::X => 0,
            Axis::Y => 1,
            Axis::Z => 2,
        };
        let (quarter_turns, direction) = match movement.turns.rem_euclid(4) {
            1 => (1, 1),
            2 => (2, 1),
            3 => (1, -1),
            _ => (0, 1),
        };
        for _ in 0..quarter_turns {
            moves.push(serde_json::json!({
                "axis": axis,
                "layer": movement.layer_start,
                "dir": direction
            }));
        }
    }

    Ok(serde_json::json!({
        "moves": moves,
        "moveCount": raw_move_count
    })
    .to_string())
}

struct ActiveSolve {
    request_token: String,
    control: NativeReductionControl,
}

#[derive(Default)]
struct SolveState {
    active: Mutex<Option<ActiveSolve>>,
}

impl SolveState {
    fn begin(&self, request_token: String, control: NativeReductionControl) -> Result<(), String> {
        let mut active = self
            .active
            .lock()
            .map_err(|_| "native solver state is unavailable".to_string())?;
        if let Some(previous) = active.replace(ActiveSolve {
            request_token,
            control,
        }) {
            previous.control.cancel();
        }
        Ok(())
    }

    /// Clear only the request that actually completed. A stale task must never
    /// remove a newer request's cancellation handle after a page reload/remount.
    fn finish(&self, request_token: &str) -> Result<(), String> {
        let mut active = self
            .active
            .lock()
            .map_err(|_| "native solver state is unavailable".to_string())?;
        if active
            .as_ref()
            .is_some_and(|solve| solve.request_token == request_token)
        {
            active.take();
        }
        Ok(())
    }

    fn cancel(&self, request_token: &str) -> Result<bool, String> {
        let mut active = self
            .active
            .lock()
            .map_err(|_| "native solver state is unavailable".to_string())?;
        if active
            .as_ref()
            .is_some_and(|solve| solve.request_token == request_token)
        {
            if let Some(solve) = active.take() {
                solve.control.cancel();
                return Ok(true);
            }
        }
        Ok(false)
    }
}

/// Run RCube on a native blocking thread. Only visible sticker colours cross the
/// frontend boundary; scramble history never reaches the solver.
#[tauri::command(rename_all = "camelCase")]
async fn solve_stickers(
    on_progress: Channel<serde_json::Value>,
    state: State<'_, SolveState>,
    request_token: String,
    n: usize,
    colors: Vec<u8>,
) -> Result<String, String> {
    if request_token.len() < 16 || request_token.len() > 128 {
        return Err("invalid solve request token".to_string());
    }
    if !(4..=MAX_NATIVE_CUBE_SIZE).contains(&n) || colors.len() != 6 * n * n {
        return Err("invalid cube size or sticker buffer".to_string());
    }

    let control = cube_wasm::native_reduction_control(n);
    state.begin(request_token.clone(), control.clone())?;

    let worker_control = control.clone();
    let join_result = tauri::async_runtime::spawn_blocking(move || {
        solve_with_rcube(
            n,
            &colors,
            &worker_control,
            Some(Box::new(move |percent| {
                if let Err(error) = on_progress.send(serde_json::json!({ "percent": percent })) {
                    eprintln!("failed to send RCube solve progress: {error}");
                }
            })),
        )
    })
    .await;

    // Cleanup is unconditional and token-aware, including task join failure.
    state.finish(&request_token)?;
    join_result.map_err(|error| format!("native solver task failed: {error}"))?
}

#[tauri::command(rename_all = "camelCase")]
async fn scramble_rcube(n: usize, seed: i32) -> Result<String, String> {
    if !(4..=MAX_NATIVE_CUBE_SIZE).contains(&n) {
        return Err("invalid cube size".to_string());
    }
    tauri::async_runtime::spawn_blocking(move || scramble_with_rcube(n, seed))
        .await
        .map_err(|error| format!("native scramble task failed: {error}"))?
}

#[tauri::command(rename_all = "camelCase")]
fn cancel_solve(state: State<'_, SolveState>, request_token: String) -> Result<bool, String> {
    state.cancel(&request_token)
}

fn main() {
    tauri::Builder::default()
        .manage(SolveState::default())
        .invoke_handler(tauri::generate_handler![
            solve_stickers,
            scramble_rcube,
            cancel_solve
        ])
        .run(tauri::generate_context!())
        .expect("error while running Solvaus Hybrid");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rcube_solution_returns_path_and_monotonic_progress() {
        let n = 4;
        let mut lab = cube_wasm::CubeLab::new(n);
        assert!(lab.scramble(18, 0x5eed) > 0);
        let colors = lab.face_colors(n);
        let control = cube_wasm::native_reduction_control(n);
        let progress = std::sync::Arc::new(Mutex::new(Vec::new()));
        let progress_callback = std::sync::Arc::clone(&progress);
        let result = solve_with_rcube(
            n,
            &colors,
            &control,
            Some(Box::new(move |percent| {
                progress_callback.lock().unwrap().push(percent);
            })),
        )
        .unwrap();
        let result: serde_json::Value = serde_json::from_str(&result).unwrap();

        assert_eq!(result["winner"], "rcube");
        assert_eq!(result["found"], true);
        let progress = progress.lock().unwrap();
        assert!(
            progress.len() > 1,
            "RCube progress callback did not stream updates"
        );
        assert_eq!(progress.last(), Some(&100.0));
        assert!(progress.windows(2).all(|pair| pair[0] <= pair[1]));
        assert!(progress[..progress.len() - 1]
            .iter()
            .all(|percent| *percent <= 99.0));
        drop(progress);
        assert_eq!(
            result["moveCount"].as_u64().unwrap() as usize,
            result["notation"].as_array().unwrap().len()
        );
    }

    #[test]
    fn rcube_texture_solution_replays_compact_move_list() {
        let n = TEXTURE_RENDER_THRESHOLD + 1;
        let mut source = cube_wasm::CubeLab::new(n);
        for (axis, start, dir) in [
            (0, 0, 1),
            (1, n / 2, -1),
            (2, 0, 1),
            (0, n / 2, -1),
            (1, 0, 1),
            (2, n / 2, -1),
        ] {
            for layer in start..start + n / 2 {
                source.apply_design_move(axis, layer, dir);
            }
        }
        let colors = source.face_colors(n);
        let control = cube_wasm::native_reduction_control(n);
        let result = solve_with_rcube(n, &colors, &control, None).unwrap();
        let result: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(result["solvedByRcube"], true);
        assert_eq!(result["compactMoves"], true);
        assert!(!result["moves"].as_array().unwrap().is_empty());
        assert!(result["moveCount"].as_u64().unwrap() > 0);

        let mut replay = cube_wasm::CubeLab::new(n);
        assert!(replay.load_face_colors(&colors));
        for movement in result["moves"].as_array().unwrap() {
            let axis = movement["axis"].as_u64().unwrap() as u8;
            let layer = movement["layer"].as_u64().unwrap() as usize;
            let turns = movement["turns"].as_i64().unwrap() as i32;
            let (count, dir) = match turns.rem_euclid(4) {
                1 => (1, 1),
                2 => (2, 1),
                3 => (1, -1),
                _ => (0, 1),
            };
            for _ in 0..count {
                replay.apply_design_move(axis, layer, dir);
            }
        }
        assert!(replay.is_solved());
    }

    #[test]
    fn rcube_scramble_replays_and_is_seeded() {
        assert!(scramble_with_rcube(MAX_NATIVE_CUBE_SIZE + 1, 0x5eed).is_err());
        let first = scramble_with_rcube(4, 0x5eed).unwrap();
        let second = scramble_with_rcube(4, 0x5eed).unwrap();
        let first: serde_json::Value = serde_json::from_str(&first).unwrap();
        let second: serde_json::Value = serde_json::from_str(&second).unwrap();
        assert_eq!(first, second);

        for n in [4, 11, 21, 32, 64, 101, MAX_NATIVE_CUBE_SIZE] {
            let scramble = if n == 4 {
                first.clone()
            } else {
                serde_json::from_str(&scramble_with_rcube(n, 0x5eed).unwrap()).unwrap()
            };
            let moves = scramble["moves"].as_array().unwrap();
            assert_eq!(scramble["moveCount"].as_u64().unwrap() as usize, 27 * n);
            if n == MAX_NATIVE_CUBE_SIZE {
                for movement in moves {
                    assert!(movement["layer"].as_u64().unwrap() < n as u64);
                    assert!([-1, 1].contains(&movement["dir"].as_i64().unwrap()));
                }
                continue;
            }

            let mut lab = cube_wasm::CubeLab::new(n);
            for movement in moves {
                lab.apply_design_move(
                    movement["axis"].as_u64().unwrap() as u8,
                    movement["layer"].as_u64().unwrap() as usize,
                    movement["dir"].as_i64().unwrap() as i32,
                );
            }
            assert!(!lab.is_solved());
            let colors = lab.face_colors(n);
            assert_eq!(colors.len(), 6 * n * n);
            assert!(lab.load_face_colors(&colors));
            if n != 4 {
                continue;
            }

            let control = cube_wasm::native_reduction_control(n);
            let solution = solve_with_rcube(n, &colors, &control, None).unwrap();
            let solution: serde_json::Value = serde_json::from_str(&solution).unwrap();
            for movement in solution["moves"].as_array().unwrap() {
                lab.apply_design_move(
                    movement["axis"].as_u64().unwrap() as u8,
                    movement["layer"].as_u64().unwrap() as usize,
                    movement["dir"].as_i64().unwrap() as i32,
                );
            }
            assert!(lab.is_solved());
        }
    }

    #[test]
    #[ignore = "release-mode RCube size-scaling and replay gate"]
    fn rcube_large_cube_wide_turn_states() {
        for n in [11, 21, 32, 51, 64, 101, MAX_NATIVE_CUBE_SIZE] {
            let width = n / 2;
            let mut source = cube_wasm::CubeLab::new(n);
            for (axis, start, dir) in [
                (0, 0, 1),
                (1, n - width, -1),
                (2, 0, 1),
                (0, n - width, -1),
                (1, 0, 1),
                (2, n - width, -1),
            ] {
                for layer in start..start + width {
                    source.apply_design_move(axis, layer, dir);
                }
            }
            let colors = source.face_colors(n);
            let control = NativeReductionControl::with_timeout(std::time::Duration::from_secs(30));
            let started = Instant::now();
            let result = solve_with_rcube(n, &colors, &control, None);
            eprintln!(
                "RCube N={n} browser-smoke state elapsed: {:?}",
                started.elapsed()
            );
            let result = result.expect("RCube must solve and replay the browser-smoke state");
            let result: serde_json::Value = serde_json::from_str(&result).unwrap();
            assert_eq!(result["winner"], "rcube");
        }
    }

    #[test]
    fn stale_completion_cannot_clear_or_cancel_new_request() {
        let state = SolveState::default();
        let old = cube_wasm::native_reduction_control(4);
        let new = cube_wasm::native_reduction_control(4);
        state
            .begin("old-request-token-0001".to_string(), old.clone())
            .unwrap();
        state
            .begin("new-request-token-0002".to_string(), new.clone())
            .unwrap();
        assert!(!old.should_continue(), "replacement must cancel old work");

        state.finish("old-request-token-0001").unwrap();
        assert!(
            new.should_continue(),
            "stale completion cleared new control"
        );
        assert!(!state.cancel("old-request-token-0001").unwrap());
        assert!(state.cancel("new-request-token-0002").unwrap());
        assert!(!new.should_continue());
    }
}
