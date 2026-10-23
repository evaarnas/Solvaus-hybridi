use std::path::Path;

/// Guard against shipping a stale or missing frontend.
///
/// This repo has lived in an iCloud-synced folder, where sync has silently
/// deleted `web/index.html` and scattered `" 2"` conflict duplicates. Because
/// Tauri embeds whatever is on disk at build time, that produced apps with a
/// broken/empty UI and no error. We fail the build loudly instead — a stale
/// frontend should never make it into a bundle.
fn assert_frontend_is_fresh() {
    let web = Path::new("../web");

    let index = web.join("index.html");
    let html = std::fs::read_to_string(&index).unwrap_or_else(|e| {
        panic!(
            "Cube Solver build aborted: cannot read web/index.html ({e}).\n\
             The frontend is missing (likely an iCloud sync conflict). Regenerate it \
             with `python3 tools/gen-index.py` before building."
        )
    });

    // Structural markers that only the real, fully-wired UI contains.
    for marker in [
        "wireRealSolver",
        "data-cancel-solve",
        "CubeLab",
        "solver-worker.js",
    ] {
        assert!(
            html.contains(marker),
            "Cube Solver build aborted: web/index.html is missing the `{marker}` marker — \
             it looks stale or incomplete. Regenerate it with `python3 tools/gen-index.py`."
        );
    }
    assert!(
        html.len() > 50_000,
        "Cube Solver build aborted: web/index.html is only {} bytes — almost certainly stale. \
         Regenerate it with `python3 tools/gen-index.py`.",
        html.len()
    );

    // The page imports the generated wasm-bindgen glue as a JavaScript module.
    // If either ignored build output is missing, Tauri can still serve the page
    // shell while the missing module URL falls back to HTML, leaving the cube
    // and all controls inert in the installed app.
    for asset in ["pkg/cube_wasm.js", "pkg/cube_wasm_bg.wasm"] {
        let path = web.join(asset);
        assert!(
            path.is_file() && std::fs::metadata(&path).is_ok_and(|metadata| metadata.len() > 0),
            "Cube Solver build aborted: required frontend asset {} is missing or empty. Run `python3 tools/build-wasm.py` before building the Tauri app.",
            path.display()
        );
        println!("cargo:rerun-if-changed={}", path.display());
    }

    let worker = web.join("solver-worker.js");
    let worker_src = std::fs::read_to_string(&worker).unwrap_or_else(|e| {
        panic!("Cube Solver build aborted: cannot read web/solver-worker.js ({e}).")
    });
    assert!(
        worker_src.contains("postMessage") && worker_src.contains("CubeLab"),
        "Cube Solver build aborted: web/solver-worker.js looks stale/incomplete."
    );

    // Re-run this check (and re-embed) whenever the frontend changes.
    println!("cargo:rerun-if-changed=../web/index.html");
    println!("cargo:rerun-if-changed=../web/solver-worker.js");
}

/// The authored UI intentionally uses inline style attributes extensively. Tauri's
/// default CSP asset pass adds a style nonce; CSP then ignores `unsafe-inline`, and
/// WebKit blocks every style attribute, leaving an unstyled document. Keep nonce/
/// hash rewriting for scripts, but never re-enable it for `style-src` unless the UI
/// has first been migrated completely to stylesheets/classes.
fn assert_packaged_style_policy() {
    let config = std::fs::read_to_string("tauri.conf.json")
        .expect("Cube Solver build aborted: cannot read src-tauri/tauri.conf.json");
    assert!(
        config.contains("\"dangerousDisableAssetCspModification\": [\"style-src\"]"),
        "Cube Solver build aborted: Tauri style-src nonce injection would block the UI's inline styles. Keep dangerousDisableAssetCspModification scoped to style-src."
    );
    println!("cargo:rerun-if-changed=tauri.conf.json");
}

fn build_rcube() {
    let mut build = cc::Build::new();
    build
        .cpp(true)
        .std("c++17")
        .include("rcube")
        .file("rcube/Cube.cpp")
        .file("rcube/CubeViewer.cpp")
        .file("rcube/Face.cpp")
        .file("rcube/TinyPngOut.cpp")
        .file("rcube/bridge.cpp")
        .warnings(false);

    build.compile("rcube_engine");

    for source in [
        "rcube/Cube.cpp",
        "rcube/Cube.h",
        "rcube/CubeViewer.cpp",
        "rcube/CubeViewer.h",
        "rcube/Face.cpp",
        "rcube/Face.h",
        "rcube/TinyPngOut.cpp",
        "rcube/TinyPngOut.hpp",
        "rcube/bridge.cpp",
    ] {
        println!("cargo:rerun-if-changed={source}");
    }
}

/// Package the NDK C++ runtime alongside the Android libraries. Some native
/// objects in the final Rust cdylib may retain a DT_NEEDED reference to this
/// shared runtime, and Play Store installs only load dependencies included in
/// the APK/AAB's ABI split.
fn stage_android_cpp_runtime() {
    let arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    let (abi, target_triple) = match arch.as_str() {
        "aarch64" => ("arm64-v8a", "aarch64-linux-android"),
        "arm" => ("armeabi-v7a", "arm-linux-androideabi"),
        "x86_64" => ("x86_64", "x86_64-linux-android"),
        "x86" => ("x86", "i686-linux-android"),
        _ => return,
    };

    for key in ["ANDROID_NDK_HOME", "NDK_HOME", "ANDROID_NDK_ROOT"] {
        println!("cargo:rerun-if-env-changed={key}");
    }
    println!("cargo:rerun-if-env-changed=ANDROID_HOME");

    let ndk_root = ["ANDROID_NDK_HOME", "NDK_HOME", "ANDROID_NDK_ROOT"]
        .into_iter()
        .find_map(|key| std::env::var_os(key).map(std::path::PathBuf::from))
        .or_else(|| {
            let sdk = std::env::var_os("ANDROID_HOME")?;
            let ndk_dir = Path::new(&sdk).join("ndk");
            std::fs::read_dir(ndk_dir)
                .ok()?
                .flatten()
                .filter(|entry| entry.path().is_dir())
                .map(|entry| entry.path())
                .max()
        })
        .expect("Android build requires ANDROID_NDK_HOME or NDK_HOME to stage libc++_shared.so");

    let host_tag = if cfg!(target_os = "windows") {
        "windows-x86_64"
    } else if cfg!(target_os = "macos") {
        "darwin-x86_64"
    } else {
        "linux-x86_64"
    };
    let runtime = ndk_root
        .join("toolchains/llvm/prebuilt")
        .join(host_tag)
        .join("sysroot/usr/lib")
        .join(target_triple)
        .join("libc++_shared.so");
    if !runtime.is_file() {
        panic!(
            "Android NDK C++ runtime was not found at {}",
            runtime.display()
        );
    }

    let destination = Path::new("gen/android/app/src/main/jniLibs")
        .join(abi)
        .join("libc++_shared.so");
    std::fs::create_dir_all(destination.parent().expect("runtime destination parent"))
        .expect("create Android jniLibs directory");
    std::fs::copy(&runtime, &destination).unwrap_or_else(|error| {
        panic!(
            "could not stage Android C++ runtime from {} to {}: {error}",
            runtime.display(),
            destination.display()
        )
    });
}

/// Gullicube runs as a child process (it is a self-contained program with its own
/// `main`, ~GBs of working memory and a data directory), so it is compiled to an
/// executable and placed beside the app binary together with its `data/` files.
/// A missing/unsupported C++ toolchain only disables that solver; it never fails
/// the app build.
fn build_gullicube() {
    use std::path::PathBuf;

    let android = std::env::var("CARGO_CFG_TARGET_OS").is_ok_and(|os| os == "android");
    let root = Path::new("gullicube");
    let sources = [
        "src/full/gullicube.cpp",
        "src/full/brobglue.cpp",
        "src/brob/xcube.cpp",
        "src/brob/cornersolver.cpp",
        "src/brob/wingsolver.cpp",
        "src/brob/util.cpp",
        "src/brob/facelets.cpp",
    ];
    for dir in ["src", "data"] {
        println!("cargo:rerun-if-changed=gullicube/{dir}");
    }

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    // OUT_DIR is target/<profile>/build/<pkg>-<hash>/out; the app binary is three levels up.
    let Some(profile_dir) = out_dir.ancestors().nth(3).map(Path::to_path_buf) else {
        return;
    };
    let exe_name = if cfg!(windows) {
        "gullicube.exe"
    } else {
        "gullicube"
    };
    let exe = profile_dir.join(exe_name);

    println!("cargo:rerun-if-env-changed=GULLICUBE_CXX");
    // The Rust toolchain may be MSVC, which cannot build gullicube: fall back to a
    // GCC/Clang on PATH (or GULLICUBE_CXX).
    let gnu_like = cc::Build::new()
        .cpp(true)
        .try_get_compiler()
        .ok()
        .filter(|c| c.is_like_gnu() || c.is_like_clang());
    let mut command = match gnu_like {
        Some(compiler) => compiler.to_command(),
        None => {
            let mut candidates: Vec<String> = match std::env::var("GULLICUBE_CXX") {
                Ok(c) => vec![c],
                Err(_) => vec!["g++".to_string(), "clang++".to_string()],
            };
            if cfg!(windows) {
                for root in ["C:\\msys64", "C:\\mingw64", "C:\\msys2"] {
                    for sub in ["ucrt64", "mingw64", "clang64", ""] {
                        candidates
                            .push(format!("{root}\\{sub}\\bin\\g++.exe").replace("\\\\", "\\"));
                    }
                }
            }
            let found = candidates.into_iter().find(|c| {
                std::process::Command::new(c)
                    .arg("--version")
                    .output()
                    .is_ok_and(|o| o.status.success())
            });
            let Some(found) = found else {
                println!("cargo:warning=Gullicube needs g++/clang++ (set GULLICUBE_CXX); the Gullicube solver is disabled in this build");
                return;
            };
            let mut cmd = std::process::Command::new(&found);
            if let Some(dir) = Path::new(&found)
                .parent()
                .filter(|d| !d.as_os_str().is_empty())
            {
                let mut paths = vec![dir.to_path_buf()];
                paths.extend(std::env::split_paths(
                    &std::env::var_os("PATH").unwrap_or_default(),
                ));
                if let Ok(joined) = std::env::join_paths(paths) {
                    cmd.env("PATH", joined);
                }
            }
            cmd
        }
    };
    if cfg!(target_os = "macos") && !android {
        // Apple's clang emits one fat binary, so the universal app runs on both CPUs.
        command.args(["-arch", "arm64", "-arch", "x86_64"]);
    }
    command
        .args(["-O2", "-std=c++20", "-w", "-I"])
        .arg(root.join("src/brob"))
        .arg("-I")
        .arg(root.join("src/full"));
    for source in sources {
        command.arg(root.join(source));
    }
    let built = out_dir.join(exe_name);
    command.arg("-o").arg(&built);
    if cfg!(windows) {
        command.arg("-static");
    }
    if android {
        // Bionic has pthreads built in; link libc++ statically so the executable is standalone.
        command.args(["-static-libstdc++", "-pie"]);
    } else {
        command.arg("-lpthread");
    }
    match command.output() {
        Ok(output) if output.status.success() => {}
        other => {
            let detail = match &other {
                Ok(o) => String::from_utf8_lossy(&o.stderr)
                    .lines()
                    .take(6)
                    .collect::<Vec<_>>()
                    .join(" | "),
                Err(e) => e.to_string(),
            };
            println!("cargo:warning=Gullicube failed to build ({detail}); the Gullicube solver is disabled in this build");
            return;
        }
    }

    // A running app keeps the old executable locked; keep it rather than failing the build.
    if android {
        // Android only executes files from the app's native library directory, so ship the
        // executable as a "library" in jniLibs (the data files are embedded in the app).
        let abi = match std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
            Ok("aarch64") => "arm64-v8a",
            Ok("arm") => "armeabi-v7a",
            Ok("x86_64") => "x86_64",
            Ok("x86") => "x86",
            _ => return,
        };
        let jni = Path::new("gen/android/app/src/main/jniLibs").join(abi);
        if std::fs::create_dir_all(&jni).is_ok() {
            if let Err(error) = std::fs::copy(&built, jni.join("libgullicube.so")) {
                println!("cargo:warning=could not stage Gullicube for Android: {error}");
            }
        }
        return;
    }
    if let Err(error) = std::fs::copy(&built, &exe) {
        println!(
            "cargo:warning=could not update {} ({error}); close the running app and rebuild",
            exe.display()
        );
    }

    copy_data(&root.join("data"), &profile_dir.join("data"));

    // Stage the executable and data for the installer (see bundle.resources).
    let stage = Path::new(BUNDLE_STAGE);
    if std::fs::create_dir_all(stage).is_ok() {
        let _ = std::fs::copy(&built, stage.join(exe_name));
        copy_data(&root.join("data"), &stage.join("data"));
    }
}

const BUNDLE_STAGE: &str = "gullicube-bundle";

fn copy_data(from: &Path, to: &Path) {
    if std::fs::create_dir_all(to).is_err() {
        return;
    }
    if let Ok(entries) = std::fs::read_dir(from) {
        for entry in entries.flatten() {
            let target = to.join(entry.file_name());
            let up_to_date = target
                .metadata()
                .ok()
                .zip(entry.metadata().ok())
                .is_some_and(|(a, b)| a.len() == b.len());
            if !up_to_date {
                let _ = std::fs::copy(entry.path(), target);
            }
        }
    }
}

fn main() {
    assert_frontend_is_fresh();
    assert_packaged_style_policy();
    build_rcube();
    if std::env::var("CARGO_CFG_TARGET_OS").is_ok_and(|os| os == "android") {
        stage_android_cpp_runtime();
    }
    // The bundle resource directory must exist even when Gullicube cannot be built.
    let _ = std::fs::create_dir_all(BUNDLE_STAGE);
    let _ = std::fs::write(
        Path::new(BUNDLE_STAGE).join("README.txt"),
        "Staged by build.rs\n",
    );
    // Gullicube cannot be built for iOS (no child processes); on Android it ships as a jniLib.
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os != "ios" {
        build_gullicube();
    }
    tauri_build::build()
}
