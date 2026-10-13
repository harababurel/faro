//! The web demo's files in the site build (WEBSITE.md §6.8, Phase 4):
//! `crates/faro-web/build.sh`'s output copied into `<out>/demo/` under
//! content-hashed names, plus the `demo/manifest.json` the page reads before
//! preloading the demo — and the `demo/manifest.no-preload.json` nginx can
//! swap in to stop preloading without a redeploy (§6.8's emergency switch).
//! Neither manifest is precompressed by `site/build.sh`: nginx's
//! `gzip_static` would otherwise keep serving a stale twin after someone
//! edits the file on disk.
//!
//! The landing page doesn't reference the demo yet (the next chunk wires it
//! up), so these files stay out of the first-load budget; `site/check.py`
//! holds the wasm to its own (D119: 1.5 MB gzipped).

#![expect(
    clippy::disallowed_methods,
    reason = "the site generator is a one-shot CLI: blocking fs is how it works"
)]

use std::fs;
use std::io::Write as _;
use std::path::Path;

use crate::assets;

/// wasm-bindgen's two files, as `crates/faro-web/build.sh` writes them.
const JS: &str = "faro_web.js";
const WASM: &str = "faro_web_bg.wasm";

/// Copy the built module in `pkg` into `<out>/demo/` with content-hashed
/// names and write the two manifests — `demo/manifest.json` and the
/// `preload: false` twin nginx swaps in to stop preloading (§6.8):
///
/// ```json
/// {"preload": true, "js": "/demo/faro_web.<hash>.js",
///  "wasm": "/demo/faro_web_bg.<hash>.wasm", "bytes": <wasm gzipped>,
///  "size": <wasm as is>}
/// ```
///
/// The loader's fallback wasm name (`new URL('faro_web_bg.wasm',
/// import.meta.url)`) is rewritten to the hashed one, so the page only has to
/// load the JS it's given. `bytes` is what nginx sends for the wasm — the
/// build's `.gz` twin (`site/build.sh`) — so the page's progress line is
/// honest. `size` is the wasm as is: what the page's reader counts, since the
/// browser decompresses as it reads (D124).
pub fn copy(pkg: &Path, out: &Path) -> Result<(), String> {
    let wasm = read(&pkg.join(WASM))?;
    let js =
        fs::read_to_string(pkg.join(JS)).map_err(|error| io_err("read", &pkg.join(JS), &error))?;
    if js.contains("{{") {
        return Err(format!(
            "{}: contains \"{{{{\": the demo's JS is copied, not filled",
            pkg.display()
        ));
    }
    let wasm_name = assets::hashed_name(&format!("demo/{WASM}"), &assets::hash_prefix(&wasm));
    let wasm_file = wasm_name.rsplit('/').next().unwrap_or(WASM);
    // The loader looks for the wasm next to the JS, by its plain name.
    let refs = assets::Refs::from([(WASM.to_owned(), wasm_file.to_owned())]);
    let js = assets::rewrite(&js, &refs);
    if !js.contains(wasm_file) {
        return Err(format!(
            "{}: never names {WASM}: the loader's wasm reference has changed shape",
            pkg.join(JS).display()
        ));
    }
    let js_name = assets::hashed_name(&format!("demo/{JS}"), &assets::hash_prefix(js.as_bytes()));
    let bytes = gzipped(&wasm)?;

    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    for (rel, preload) in [
        ("demo/manifest.json", true),
        // nginx's emergency switch (§6.8): uncommenting its `location` line
        // serves this instead, with no rebuild and no hashed name to change.
        ("demo/manifest.no-preload.json", false),
    ] {
        let text = manifest(preload, &js_name, &wasm_name, bytes, wasm.len());
        files.push((rel.to_owned(), text.into_bytes()));
    }
    files.push((js_name, js.into_bytes()));
    files.push((wasm_name, wasm));

    for (rel, content) in files {
        let path = out.join(&rel);
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).map_err(|error| io_err("create", dir, &error))?;
        }
        fs::write(&path, content).map_err(|error| io_err("write", &path, &error))?;
    }
    Ok(())
}

/// One manifest as the page reads it: the same fields, `preload` as given.
fn manifest(preload: bool, js: &str, wasm: &str, bytes: usize, size: usize) -> String {
    format!(
        r#"{{"preload":{preload},"js":"/{js}","wasm":"/{wasm}","bytes":{bytes},"size":{size}}}"#
    )
}

/// The wasm gzipped at the best level, as `gzip -9` in `site/build.sh` does.
fn gzipped(bytes: &[u8]) -> Result<usize, String> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
    encoder
        .write_all(bytes)
        .map_err(|error| format!("gzipping the demo's wasm failed: {error}"))?;
    encoder
        .finish()
        .map(|compressed| compressed.len())
        .map_err(|error| format!("gzipping the demo's wasm failed: {error}"))
}

fn read(path: &Path) -> Result<Vec<u8>, String> {
    fs::read(path).map_err(|error| io_err("read", path, &error))
}

fn io_err(action: &str, path: &Path, error: &std::io::Error) -> String {
    format!("failed to {action} {}: {error}", path.display())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn has_ext(name: &str, ext: &str) -> bool {
        Path::new(name)
            .extension()
            .is_some_and(|found| found == ext)
    }

    /// A stand-in for wasm-bindgen's output: the loader's fallback name is
    /// what has to be rewritten.
    fn fake_pkg(dir: &Path) {
        fs::create_dir_all(dir).expect("the fake pkg dir");
        fs::write(
            dir.join(WASM),
            b"wasm bytes, quite long to compress a little",
        )
        .expect("wasm");
        fs::write(
            dir.join(JS),
            b"async function init(module_or_path) {\
                if (module_or_path === undefined) {\
                    module_or_path = new URL('faro_web_bg.wasm', import.meta.url);\
                }\
                const response = await fetch(module_or_path);\
            }",
        )
        .expect("js");
    }

    #[test]
    fn the_demo_files_are_hashed_and_the_manifest_points_at_them() {
        let root = std::env::temp_dir().join(format!("faro-site-demo-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let pkg = root.join("pkg");
        let out = root.join("out");
        fake_pkg(&pkg);
        copy(&pkg, &out).expect("the demo files are copied");

        let names: Vec<String> = fs::read_dir(out.join("demo"))
            .expect("the demo dir")
            .filter_map(|entry| {
                entry
                    .ok()
                    .map(|e| e.file_name().to_string_lossy().into_owned())
            })
            .collect();
        let js = names
            .iter()
            .find(|name| name.starts_with("faro_web.") && has_ext(name, "js"))
            .expect("a hashed js");
        let wasm = names
            .iter()
            .find(|name| name.starts_with("faro_web_bg.") && has_ext(name, "wasm"))
            .expect("a hashed wasm");
        assert!(
            names.iter().any(|name| name == "manifest.json"),
            "{names:?}"
        );
        assert!(
            names.iter().any(|name| name == "manifest.no-preload.json"),
            "{names:?}"
        );
        assert!(
            !names.iter().any(|name| name == JS),
            "the plain name is gone"
        );

        // The loader now finds the hashed wasm beside it.
        let js_text = fs::read_to_string(out.join("demo").join(js)).expect("the hashed js");
        assert!(
            js_text.contains(&format!("new URL('{wasm}', import.meta.url)")),
            "{js_text}"
        );
        assert!(!js_text.contains("'faro_web_bg.wasm'"), "{js_text}");

        // Both manifests: absolute hashed URLs, the wasm's wire size
        // (§6.8), and preloading on in one, off in the other.
        for (rel, preload) in [
            ("demo/manifest.json", true),
            ("demo/manifest.no-preload.json", false),
        ] {
            let text = fs::read_to_string(out.join(rel)).expect("the manifest");
            let data: serde_json::Value = serde_json::from_str(&text).expect("valid JSON");
            assert_eq!(data["preload"], serde_json::Value::Bool(preload), "{text}");
            assert_eq!(data["js"], format!("/demo/{js}"), "{text}");
            assert_eq!(data["wasm"], format!("/demo/{wasm}"), "{text}");
            let bytes = data["bytes"].as_u64().expect("a size");
            assert!(bytes > 0 && bytes < 200, "{text}");
            assert_eq!(data["size"], 43, "the wasm as is: {text}");
        }
        let _ = fs::remove_dir_all(&root);
    }
}
