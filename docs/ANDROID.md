# Android app

The desktop Tauri shell also builds for Android (Tauri 2 mobile). The WebView loads the same
`web/` frontend, which applies its compact `html.android` layout.

- **Solvers:** [RCube](https://github.com/ShellPuppy/RCube) is compiled by the NDK and works natively. [Gullicube](https://github.com/rokicki/gullicube/) is compiled by `build.rs`
  into `libgullicube.so` (an executable placed in `gen/android/app/src/main/jniLibs/<abi>/`, the only
  location Android lets apps run files from); its data tables are embedded in the app and unpacked to
  private storage on first use. The CI workflow enables `useLegacyPackaging` for native libraries so
  the executable is extracted to disk. Gullicube needs a 64-bit target (`__int128`), so it is
  available on arm64 phones but shows as "not installed" in the 32-bit armv7 build. It can need about
  3 GB of RAM, so on phones only small and medium cubes are likely to finish; Android may stop the
  process on larger ones.
- **Build in CI:** the `Android app` workflow produces a debug-signed APK (arm64 + armv7) that can be
  sideloaded (`adb install` or opening the file on the phone).
- **Build locally:** install JDK 17, the Android SDK and NDK (set `ANDROID_HOME` and `NDK_HOME`), add
  the Rust targets `aarch64-linux-android armv7-linux-androideabi`, then from `src-tauri/`:

  ```sh
  cargo tauri android init
  python ../tools/configure-android.py
  cargo tauri android build --debug --features custom-protocol --apk
  ```

  To instrument release or debug WebView logging, run that build once first so Tauri generates
  its Kotlin sources; then run `python ../tools/patch-android-webview-logging.py` and rebuild.

- **Play Store build:** build a release app bundle with the custom protocol enabled:

  Run `prepare-play-store.bat` from the repository root. After preparing the frontend and
  generating `gen/android`, it starts `cargo tauri android dev --open`; keep that terminal window
  running while Android Studio builds. The generated Gradle Rust tasks call back to the Tauri CLI
  for build options, so opening `gen/android` directly does not start that options server. The
  script does not build an AAB.

  ```sh
  cargo tauri android build --features custom-protocol --aab --target aarch64 --target armv7
  ```

  The Tauri build hook generates the WASM package automatically. To verify or build it
  explicitly first, run `python ../tools/build-wasm.py` from `src-tauri/`; the native build now
  fails early if `web/pkg/cube_wasm.js` or `web/pkg/cube_wasm_bg.wasm` is missing.

- **WebView diagnostics:** the patch above adds `SolvausWebView` request and HTTP errors, plus
  `SolvausWebConsole` JavaScript errors (including module import and WebGL startup failures) in
  release builds. Rebuild and install that APK, reproduce the blank cube, then capture the matching
  lines with `adb logcat -s SolvausWebView:V SolvausWebConsole:E`.

- **Release signing:** for Play Store or a release APK, create a keystore and follow
  <https://v2.tauri.app/distribute/sign/android/>.

Very large cubes (hundreds of layers) can exceed a phone's memory; prefer sizes up to about 100.

## Reinstall / update fails

Android only updates an app if it is signed with the same key. APKs from CI runs before the stable debug key was added (and any build made on another machine) use a different key, so uninstall the old app once, then install the new APK. Later CI builds reuse one cached key and update in place. If the cache is evicted (unused for 7 days), uninstall once more.
