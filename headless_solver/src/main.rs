use std::fs::OpenOptions;
use std::io::Write;
use std::time::Instant;

use cube_core::{Challenge, ChallengeSpec, CubeSize, CubeSnapshot, CubeState, StickerCube};
use cube_solver::kociemba::search::Solver;

fn main() {
    let mut args = std::env::args();
    args.next();

    match args.next().as_deref() {
        Some("benchmark") => benchmark(),
        Some(size) => {
            let size: usize = size.parse().expect("size must be a number");
            let input = args.next().expect("usage: headless_solver <size> <state>");

            solve_state(size, &input);
        }
        _ => {
            eprintln!("usage:");
            eprintln!("  headless_solver <size> <state>");
            eprintln!("  headless_solver benchmark");
            std::process::exit(1);
        }
    }
}

fn benchmark() {
    let mut csv = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open("benchmark.csv")
        .expect("failed to open benchmark.csv");

    writeln!(csv, "size,seed,moves,seconds").unwrap();

    println!("Building Kociemba tables...");
    let solver = Solver::new();

    for size in 3..=21 {
        println!();
        println!("=== {}x{}x{} ===", size, size, size);

        let size = CubeSize::new(size).unwrap();

        let seed = rand::random::<u64>();

        let challenge = Challenge::generate(
            size,
            ChallengeSpec {
                seed,
                scramble_depth: 1000,
                max_layer_span: if size.get() == 3 { 1 } else { size.get() },
            },
        )
        .expect("failed to generate scramble");

        let mut cube = challenge.into_cube();

        cube.validate().expect("generated cube is invalid");

        let start = Instant::now();

        let result = if size.get() == 3 {
            cube_solver::kociemba::cube3::solve_sticker(&cube, &solver)
        } else {
            cube_solver::reduction::solve_reduction(&mut cube, &solver)
        };

        match result {
            Some(moves) => {
                let elapsed = start.elapsed();
                let seconds = elapsed.as_secs_f64();

                // println!("Seed: {}", seed);
                println!("Solved in {} moves.", moves.len());
                println!("Solve time: {:.3} seconds.", seconds);

                writeln!(csv, "{},{},{},{}", size.get(), seed, moves.len(), seconds).unwrap();

                csv.flush().unwrap();
            }
            None => {
                eprintln!("Solver failed.");
                std::process::exit(1);
            }
        }
    }
}

fn solve_state(size: usize, input: &str) {
    let stickers_per_face = size * size;
    let expected_length = stickers_per_face * 6;

    if input.len() != expected_length {
        eprintln!(
            "Expected {} characters for {}x{}x{}, got {}",
            expected_length,
            size,
            size,
            size,
            input.len()
        );
        std::process::exit(1);
    }

    // psolver: U F R B L D
    // Cube-Solver: U D F B L R
    let faces = [
        &input[0..stickers_per_face],                         // U
        &input[5 * stickers_per_face..6 * stickers_per_face], // D
        &input[stickers_per_face..2 * stickers_per_face],     // F
        &input[3 * stickers_per_face..4 * stickers_per_face], // B
        &input[4 * stickers_per_face..5 * stickers_per_face], // L
        &input[2 * stickers_per_face..3 * stickers_per_face], // R
    ];

    let colors: Vec<u8> = faces
        .iter()
        .flat_map(|face| face.bytes())
        .map(|c| match c.to_ascii_uppercase() {
            b'W' => 0,
            b'Y' => 1,
            b'G' => 2,
            b'B' => 3,
            b'O' => 4,
            b'R' => 5,
            _ => {
                eprintln!("Invalid sticker character: {}", c as char);
                std::process::exit(1);
            }
        })
        .collect();

    let json = serde_json::json!({
        "size": size,
        "stickers": colors.iter().map(|&c| match c {
            0 => "White",
            1 => "Yellow",
            2 => "Green",
            3 => "Blue",
            4 => "Orange",
            5 => "Red",
            _ => unreachable!(),
        }).collect::<Vec<_>>()
    });

    let snapshot: CubeSnapshot =
        serde_json::from_value(json).expect("failed to construct CubeSnapshot");

    let mut cube = StickerCube::from_snapshot(snapshot);

    cube.validate().expect("invalid / impossible cube state");

    println!("Building Kociemba tables...");
    let solver = Solver::new();

    println!("Starting {}x{}x{} reduction solve...", size, size, size);

    let start = Instant::now();

    match cube_solver::reduction::solve_reduction(&mut cube, &solver) {
        Some(moves) => {
            let elapsed = start.elapsed();

            println!("Solved in {} moves.", moves.len());
            println!("Solve time: {:.3} seconds.", elapsed.as_secs_f64());
        }
        None => {
            eprintln!("Reduction solver failed.");
            std::process::exit(1);
        }
    }
}
