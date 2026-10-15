# Android app

The desktop Tauri shell also builds for Android (Tauri 2 mobile). The WebView loads the same
`web/` frontend, which applies its compact `html.android` layout.

- **Solvers:** RCube is compiled by the NDK and works natively. Gullicube is a host executable
  and is not shipped on Android; the UI shows it as "not installed".
- **Build in CI:** the `Android app` workflow produces a debug-signed APK (arm64 + armv7) that can be
  sideloaded (`adb install` or opening the file on the phone).
- **Build locally:** install JDK 17, the Android SDK and NDK (set `ANDROID_HOME` and `NDK_HOME`), add
  the Rust targets `aarch64-linux-android armv7-linux-androideabi`, then from `src-tauri/`:

  ```sh
  cargo tauri android init
  cargo tauri android build --debug --apk
  ```

- **Release signing:** for Play Store or a release APK, create a keystore and follow
  <https://v2.tauri.app/distribute/sign/android/>.

Very large cubes (hundreds of layers) can exceed a phone's memory; prefer sizes up to about 100.

## Reinstall / update fails

Android only updates an app if it is signed with the same key. APKs from CI runs before the stable debug key was added (and any build made on another machine) use a different key, so uninstall the old app once, then install the new APK. Later CI builds reuse one cached key and update in place. If the cache is evicted (unused for 7 days), uninstall once more.
