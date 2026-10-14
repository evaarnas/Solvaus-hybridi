//! Gullicube backend: an external N×N×N solver run as a child process.
//!
//! The executable is located at run time (see [`locate`]); it is not linked into
//! the app, so the desktop build works without it and reports a clear error when
//! the solver is missing. Only the visible sticker state is piped to it as a
//! Kociemba-style facelet string.

use cube_core::{Axis, Move};
use cube_wasm::NativeReductionControl;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const EXE_NAME: &str = if cfg!(windows) {
    "gullicube.exe"
} else {
    "gullicube"
};

/// Face blocks of `Face::ALL` order (U, D, F, B, L, R) in the order gullicube
/// expects them: U R F D L B.
const FACELET_FACE_ORDER: [usize; 6] = [0, 5, 2, 1, 4, 3];
/// Colour index (== solved face index in `Face::ALL` order) to facelet letter.
const FACELET_LETTERS: [u8; 6] = *b"UDFBLR";

pub const MISSING_MESSAGE: &str =
    "Gullicube was not found: install g++ or clang++ (on Windows, MSYS2 MinGW) and rebuild the app, or set SOLVAUS_GULLICUBE to a prebuilt executable";

/// Practical ceiling for the app: gullicube needs about 3 GB and is time-budgeted.
pub const MAX_SIZE: usize = 1001;

fn candidate_dirs() -> Vec<PathBuf> {
    let mut starts = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            starts.push(dir.to_path_buf());
            // Tauri resource locations: macOS .app and Linux deb/Flatpak.
            if let Some(parent) = dir.parent() {
                starts.push(parent.join("Resources"));
                starts.push(parent.join("lib").join("cube-solver"));
            }
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        starts.push(cwd);
    }
    let mut dirs = Vec::new();
    for start in starts {
        for ancestor in start.ancestors().take(6) {
            dirs.push(ancestor.to_path_buf());
            dirs.push(ancestor.join("gullicube"));
        }
    }
    dirs
}

/// `SOLVAUS_GULLICUBE` (path to the executable), else `gullicube[.exe]` in or
/// beside the app directory or any of its first few ancestors, else a sibling
/// `gullicube/` checkout of the repository.
pub fn locate() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("SOLVAUS_GULLICUBE") {
        let path = PathBuf::from(path);
        return path.is_file().then_some(path);
    }
    candidate_dirs()
        .into_iter()
        .map(|dir| dir.join(EXE_NAME))
        .find(|path| path.is_file() && path.parent().is_some_and(|dir| dir.join("data").is_dir()))
}

pub fn is_available() -> bool {
    locate().is_some()
}

pub fn facelets(n: usize, colors: &[u8]) -> Result<String, String> {
    let area = n * n;
    if colors.len() != 6 * area {
        return Err("invalid sticker buffer".to_string());
    }
    let mut out = String::with_capacity(6 * area);
    for block in FACELET_FACE_ORDER {
        for &color in &colors[block * area..(block + 1) * area] {
            let letter = FACELET_LETTERS
                .get(usize::from(color))
                .ok_or_else(|| "invalid sticker colour".to_string())?;
            out.push(char::from(*letter));
        }
    }
    Ok(out)
}

/// Parse one gullicube move such as `U`, `R2`, `D'` or `3L2` into a core move.
pub fn parse_move(token: &str, n: usize) -> Option<Move> {
    let bytes = token.as_bytes();
    let face_at = bytes.iter().position(|b| b.is_ascii_alphabetic())?;
    let depth = if face_at == 0 {
        1
    } else {
        token[..face_at].parse::<usize>().ok()?
    };
    if depth == 0 || depth > n {
        return None;
    }
    let face = bytes[face_at];
    let turns: i8 = match &token[face_at + 1..] {
        "" => 1,
        "2" => 2,
        "'" => -1,
        _ => return None,
    };
    let (axis, layer, sign) = match face {
        b'U' => (Axis::Y, n - depth, -1),
        b'D' => (Axis::Y, depth - 1, 1),
        b'R' => (Axis::X, n - depth, -1),
        b'L' => (Axis::X, depth - 1, 1),
        b'F' => (Axis::Z, n - depth, -1),
        b'B' => (Axis::Z, depth - 1, 1),
        _ => return None,
    };
    Some(Move::new(axis, layer, layer, turns * sign))
}

/// A solution line starts with a space and holds nothing but move tokens; the
/// solver's other output lines never parse as moves.
fn parse_solution_line(line: &str, n: usize, moves: &mut Vec<Move>) {
    if !line.starts_with(' ') || line.starts_with("  ") || line.trim().is_empty() {
        return;
    }
    let parsed: Option<Vec<Move>> = line
        .split_whitespace()
        .map(|token| parse_move(token, n))
        .collect();
    if let Some(parsed) = parsed {
        moves.extend(parsed);
    }
}

pub fn solve(
    n: usize,
    colors: &[u8],
    control: &NativeReductionControl,
    progress: Option<&(dyn Fn(f64) + Send + Sync)>,
) -> Result<Vec<Move>, String> {
    let exe = locate().ok_or_else(|| MISSING_MESSAGE.to_string())?;
    let facelets = facelets(n, colors)?;
    let budget = 10.0 + 50.0 * n as f64 / 1024.0;

    let mut command = Command::new(&exe);
    command
        .arg(n.to_string())
        .args(["-f", "-o"])
        .current_dir(exe.parent().unwrap_or_else(|| Path::new(".")))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut child = command
        .spawn()
        .map_err(|error| format!("could not start Gullicube: {error}"))?;

    let mut stdin = child.stdin.take().ok_or("Gullicube stdin unavailable")?;
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(facelets.as_bytes());
    });
    let stdout = child.stdout.take().ok_or("Gullicube stdout unavailable")?;
    let mut stderr = child.stderr.take().ok_or("Gullicube stderr unavailable")?;
    let reader = std::thread::spawn(move || {
        let mut moves = Vec::new();
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            parse_solution_line(&line, n, &mut moves);
        }
        moves
    });
    let error_reader = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = stderr.by_ref().take(4096).read_to_string(&mut text);
        let _ = std::io::copy(&mut stderr, &mut std::io::sink());
        text
    });

    let started = Instant::now();
    let mut last_report = Instant::now() - Duration::from_secs(1);
    let status = loop {
        if !control.should_continue() {
            let _ = child.kill();
            let _ = child.wait();
            let _ = (writer.join(), reader.join(), error_reader.join());
            return Err("solve cancelled or timed out".to_string());
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {}
            Err(error) => return Err(format!("Gullicube failed: {error}")),
        }
        if let Some(report) = progress {
            if last_report.elapsed() >= Duration::from_millis(500) {
                last_report = Instant::now();
                report((started.elapsed().as_secs_f64() / (budget + 3.0) * 100.0).min(95.0));
            }
        }
        std::thread::sleep(Duration::from_millis(25));
    };

    let _ = writer.join();
    let moves = reader
        .join()
        .map_err(|_| "Gullicube output reader failed")?;
    let stderr_text = error_reader.join().unwrap_or_default();
    if !status.success() {
        let detail = stderr_text.lines().next().unwrap_or("").trim();
        return Err(if detail.is_empty() {
            format!("Gullicube exited with {status}")
        } else {
            format!("Gullicube failed: {detail}")
        });
    }
    if let Some(report) = progress {
        report(100.0);
    }
    Ok(moves
        .into_iter()
        .filter(|movement| !movement.is_noop())
        .collect())
}
