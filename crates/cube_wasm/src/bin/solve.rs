use std::env;

fn main() {
    let input = env::args()
        .nth(1)
        .expect("usage: cargo run -p cube_wasm --bin solve -- <150-character-state>");

    if input.len() != 150 {
        eprintln!("Expected 150 characters, got {}", input.len());
        std::process::exit(1);
    }

    // psolver uses U F R B L D.
    // Cube-Solver uses U D F B L R.
    let faces = [
        &input[0..25],    // U
        &input[125..150], // D
        &input[25..50],   // F
        &input[75..100],  // B
        &input[100..125], // L
        &input[50..75],   // R
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

    println!("Starting 5x5 reduction solve...");

    let control = cube_wasm::native_reduction_control(5);

    match cube_wasm::solve_reduction_sticker_state(5, &colors, &control) {
        Ok(solution) => println!("{solution}"),
        Err(error) => {
            eprintln!("Solver failed: {error}");
            std::process::exit(1);
        }
    }
}
