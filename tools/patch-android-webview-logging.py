#!/usr/bin/env python3
"""Add focused WebView diagnostics to Tauri's generated Android clients."""

from __future__ import annotations

from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CLIENT = (
    ROOT / "src-tauri" / "gen" / "android" / "app" / "src" / "main" / "java"
    / "com" / "gmail" / "solvaus" / "hybridi" / "generated" / "RustWebViewClient.kt"
)
CHROME_CLIENT = CLIENT.with_name("RustWebChromeClient.kt")


def patch_webview_client() -> None:
    if not CLIENT.is_file():
        raise SystemExit(f"Tauri Android WebView client not found: {CLIENT}")
    source = CLIENT.read_text(encoding="utf-8")
    if 'Log.e("SolvausWebView", "Request failed:' in source and "Rust handler returned" in source:
        return

    imports = "import android.os.Looper\n"
    intercept = (
        "        lastInterceptedUrl = request.url\n"
        "        return if (Rust.withAssetLoader((view as RustWebView).id)) {\n"
        "            assetLoader.shouldInterceptRequest(request.url)\n"
        "        } else {\n"
        "            val response = Rust.handleRequest(view.id, request, view.isDocumentStartScriptEnabled)\n"
    )
    if imports not in source or intercept not in source:
        raise SystemExit("Unrecognized Tauri RustWebViewClient.kt template; refusing to patch it.")
    source = source.replace(imports, imports + "import android.util.Log\n", 1)
    source = source.replace(
        "import androidx.webkit.WebViewAssetLoader\n",
        "import androidx.webkit.WebViewAssetLoader\nimport java.io.ByteArrayInputStream\n", 1,
    )
    replacement = '''        lastInterceptedUrl = request.url
        val webView = view as RustWebView
        val useAssetLoader = Rust.withAssetLoader(webView.id)
        if (request.isForMainFrame) {
            Log.i("SolvausWebView", "Main request ${request.url}; assetLoader=$useAssetLoader")
        }
        return if (useAssetLoader) {
            val response = assetLoader.shouldInterceptRequest(request.url)
            if (request.isForMainFrame) {
                Log.i("SolvausWebView", "Asset loader returned response=${response != null}")
            }
            response
        } else {
            val response = Rust.handleRequest(view.id, request, view.isDocumentStartScriptEnabled)
            if (response != null && response.statusCode >= 400) {
                val body = response.data?.readBytes()
                if (body != null) {
                    response.data = ByteArrayInputStream(body)
                    Log.e("SolvausWebView", "Rust handler returned ${response.statusCode} for ${request.url}: " +
                        String(body, Charsets.UTF_8).take(2000))
                } else {
                    Log.e("SolvausWebView", "Rust handler returned ${response.statusCode} for ${request.url} with no body")
                }
            }
'''.replace("\n+", "\n")
    source = source.replace(intercept, replacement, 1)
    error_signature = (
        "    override fun onReceivedError(\n"
        "        view: WebView,\n"
        "        request: WebResourceRequest,\n"
        "        error: WebResourceError\n"
        "    ) {\n"
    )
    if error_signature not in source:
        raise SystemExit("Unrecognized onReceivedError signature; refusing to patch it.")
    source = source.replace(
        error_signature,
        error_signature
        + '        Log.e("SolvausWebView", "Request failed: url=${request.url}, " +\n'
        + '            "mainFrame=${request.isForMainFrame}, code=${error.errorCode}, " +\n'
        + '            "description=${error.description}")\n', 1,
    )
    ending = "\n\n    \n}"
    if "override fun onReceivedHttpError(" not in source:
        if ending not in source:
            raise SystemExit("Unrecognized RustWebViewClient.kt ending; refusing to patch HTTP errors.")
        source = source.replace(
            ending,
            '\n\n    override fun onReceivedHttpError(\n'
            '        view: WebView,\n'
            '        request: WebResourceRequest,\n'
            '        errorResponse: WebResourceResponse\n'
            '    ) {\n'
            '        Log.e("SolvausWebView", "HTTP error: url=${request.url}, " +\n'
            '            "mainFrame=${request.isForMainFrame}, status=${errorResponse.statusCode}, " +\n'
            '            "reason=${errorResponse.reasonPhrase}")\n'
            '        super.onReceivedHttpError(view, request, errorResponse)\n'
            '    }\n' + ending, 1,
        )
    CLIENT.write_text(source, encoding="utf-8", newline="\n")


def patch_chrome_client() -> None:
    if not CHROME_CLIENT.is_file():
        raise SystemExit(f"Tauri Android WebChrome client not found: {CHROME_CLIENT}")
    source = CHROME_CLIENT.read_text(encoding="utf-8")
    if 'Log.e("SolvausWebConsole", msg)' in source:
        return
    import_marker = "import android.webkit.*\n"
    error_marker = '      if ("ERROR".equals(level, ignoreCase = true)) {\n        Logger.error(tag, msg, null)'
    if import_marker not in source or error_marker not in source:
        raise SystemExit("Unrecognized Tauri RustWebChromeClient.kt template; refusing to patch it.")
    if "import android.util.Log\n" not in source:
        source = source.replace(import_marker, import_marker + "import android.util.Log\n", 1)
    source = source.replace(
        error_marker,
        '      if ("ERROR".equals(level, ignoreCase = true)) {\n'
        '        Log.e("SolvausWebConsole", msg)\n'
        '        Logger.error(tag, msg, null)', 1,
    )
    CHROME_CLIENT.write_text(source, encoding="utf-8", newline="\n")


def main() -> int:
    patch_webview_client()
    patch_chrome_client()
    print(f"WebView request and console diagnostics are present under {CLIENT.parent}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
