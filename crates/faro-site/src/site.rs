//! Reading the template, transforming it and writing the site: the only
//! part of the generator that touches the file system.

#![expect(
    clippy::disallowed_methods,
    reason = "the site generator is a one-shot CLI: blocking fs is how it works"
)]

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use faro_app::app::keymap::Keymap;

use crate::assets;
use crate::demo;
use crate::keys;
use crate::placeholders::{self, Problem, Syntax, Values};
use crate::release;
use crate::scene;
use crate::themes;

/// Font extensions: these get content-hashed names like CSS and JS do.
const FONT_EXTS: [&str; 4] = ["woff", "woff2", "ttf", "otf"];

/// What happens to a template file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Font,
    Css,
    Js,
    Html,
    /// Under the `js/` path nginx proxies or serves under fixed names
    /// (`/js/s.js`, `/js/init.js`): never renamed.
    Fixed,
    /// Filled like HTML but with escaped values, and kept under its plain
    /// name: sitemaps are fetched at a fixed URL.
    Xml,
    Copy,
}

/// Build the site: copy the template, fill the placeholders from faro's
/// own code, and rename the assets to content-hashed names. With `pkg`,
/// the web demo's built wasm module (the output of
/// `crates/faro-web/build.sh`) goes into `<out>/demo/` as well.
pub fn generate(template: &Path, out: &Path, pkg: Option<&Path>) -> Result<(), String> {
    let mut found: Vec<(String, Vec<u8>)> = Vec::new();
    walk(template, "", &mut found)?;
    let contents: BTreeMap<String, Vec<u8>> = found.into_iter().collect();

    let art = scene::render()?;
    let og_name = assets::hashed_name("og.png", &assets::hash_prefix(&art.og_png));
    let values = values(&art.posters, format!("https://getfaro.dev/{og_name}"))?;

    // Hashed assets, in dependency order: the §4.7 phone frames and the
    // fonts (content unchanged) first, then CSS — hashed after its font
    // *and* phone url()s are rewritten — then JS.
    let mut hashed: BTreeMap<String, String> = BTreeMap::new();
    let mut written: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    for frame in &art.phones {
        let new = assets::hashed_name(&frame.name, &assets::hash_prefix(frame.svg.as_bytes()));
        hashed.insert(frame.name.clone(), new.clone());
        written.insert(new, frame.svg.clone().into_bytes());
    }
    for (rel, bytes) in &contents {
        if kind(rel) == Kind::Font {
            let new = assets::hashed_name(rel, &assets::hash_prefix(bytes));
            hashed.insert(rel.clone(), new.clone());
            written.insert(new, bytes.clone());
        }
    }
    for (rel, bytes) in &contents {
        if kind(rel) != Kind::Css {
            continue;
        }
        let text = utf8(rel, bytes)?;
        // A stylesheet pointing at another stylesheet would need the
        // other's hash before its own is known; the page has one.
        for other in contents.keys() {
            if other != rel && kind(other) == Kind::Css {
                let probe = assets::refs_for(
                    dir_of(rel),
                    &BTreeMap::from([(other.clone(), "X".to_owned())]),
                );
                if assets::rewrite(&text, &probe) != text {
                    return Err(format!(
                        "{rel}: references {other}: cross-stylesheet references aren't supported (the page has one style sheet)"
                    ));
                }
            }
        }
        let rewritten = assets::rewrite(&text, &assets::refs_for(dir_of(rel), &hashed));
        let filled = fill(rel, &rewritten, &values, Syntax::Css)?;
        let bytes = filled.into_bytes();
        let new = assets::hashed_name(rel, &assets::hash_prefix(&bytes));
        hashed.insert(rel.clone(), new.clone());
        written.insert(new, bytes);
    }
    for (rel, bytes) in &contents {
        if kind(rel) != Kind::Js {
            continue;
        }
        // JS is copied, not filled: rewrite its asset references, but a
        // `{{` in it is a mistake the generator can't do anything about.
        let text = utf8(rel, bytes)?;
        let rewritten = assets::rewrite(&text, &assets::refs_for(dir_of(rel), &hashed));
        if rewritten.contains("{{") {
            return Err(format!(
                "{rel}: contains \"{{{{\": JS files are copied, not filled"
            ));
        }
        let bytes = rewritten.into_bytes();
        let new = assets::hashed_name(rel, &assets::hash_prefix(&bytes));
        hashed.insert(rel.clone(), new.clone());
        written.insert(new, bytes);
    }
    for (rel, bytes) in &contents {
        if kind(rel) != Kind::Html {
            continue;
        }
        let text = utf8(rel, bytes)?;
        let refs = assets::refs_for(dir_of(rel), &hashed);
        let rewritten = assets::rewrite(&text, &refs);
        let filled = fill(rel, &rewritten, &values, Syntax::Html)?;
        written.insert(rel.clone(), filled.into_bytes());
    }
    for (rel, bytes) in &contents {
        if kind(rel) != Kind::Xml {
            continue;
        }
        // Sitemaps name pages, never hashed assets, so no reference
        // rewriting — just the placeholders, with escaped values.
        let text = utf8(rel, bytes)?;
        let filled = fill(rel, &text, &values, Syntax::Xml)?;
        written.insert(rel.clone(), filled.into_bytes());
    }
    for (rel, bytes) in &contents {
        if matches!(kind(rel), Kind::Fixed | Kind::Copy) {
            written.insert(rel.clone(), bytes.clone());
        }
    }
    written.insert(og_name, art.og_png);

    for (rel, bytes) in &written {
        let path = out.join(rel);
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).map_err(|error| io_err("create", dir, &error))?;
        }
        fs::write(&path, bytes).map_err(|error| io_err("write", &path, &error))?;
    }
    if let Some(pkg) = pkg {
        demo::copy(pkg, out)?;
    }
    Ok(())
}

/// Every value the template's placeholders can name.
fn values(posters: &scene::Posters, og_url: String) -> Result<Values, String> {
    let keymap = Keymap::default();
    // Both posters inline, each in a wrapper the CSS shows or hides per the
    // page's scheme (style.css `.poster-dark` / `.poster-light`, §3.2).
    let poster = format!(
        "<div class=\"poster-dark\">{}</div><div class=\"poster-light\">{}</div>",
        posters.dark, posters.light
    );
    let entries: [(&'static str, String); 6] = [
        ("version", release::version()),
        ("date", release::date()?),
        ("keys", keys::table(&keymap)?),
        ("themes", themes::chips()),
        ("poster", poster),
        ("og_image", og_url),
    ];
    Ok(entries.into_iter().collect())
}

/// Fill the placeholders, naming the file and what's wrong on failure.
fn fill(rel: &str, text: &str, values: &Values, syntax: Syntax) -> Result<String, String> {
    placeholders::fill(text, values, syntax).map_err(|problems: Vec<Problem>| {
        let list: Vec<String> = problems.iter().map(ToString::to_string).collect();
        format!("{rel}: {}", list.join("; "))
    })
}

/// Copy `dir` and everything under it into `files` as `(site-relative
/// path, bytes)`. Symlinks are an error: the template is a plain tree,
/// and following links could pull in anything.
fn walk(dir: &Path, prefix: &str, files: &mut Vec<(String, Vec<u8>)>) -> Result<(), String> {
    let reader = fs::read_dir(dir).map_err(|error| io_err("read", dir, &error))?;
    let mut entries: Vec<fs::DirEntry> = Vec::new();
    for entry in reader {
        entries.push(entry.map_err(|error| io_err("read", dir, &error))?);
    }
    entries.sort_by_key(fs::DirEntry::file_name);
    for entry in entries {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.is_empty() {
            continue;
        }
        let rel = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        let file_type = entry
            .file_type()
            .map_err(|error| io_err("read", &entry.path(), &error))?;
        if file_type.is_symlink() {
            return Err(format!(
                "{} is a symlink: the template must not contain symlinks",
                entry.path().display()
            ));
        }
        if file_type.is_dir() {
            walk(&entry.path(), &rel, files)?;
        } else if file_type.is_file() {
            let bytes =
                fs::read(entry.path()).map_err(|error| io_err("read", &entry.path(), &error))?;
            files.push((rel, bytes));
        } else {
            return Err(format!(
                "{} is neither a file nor a directory",
                entry.path().display()
            ));
        }
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(())
}

fn kind(rel: &str) -> Kind {
    let ext = ext_of(rel);
    if FONT_EXTS.contains(&ext) {
        Kind::Font
    } else if ext == "css" {
        Kind::Css
    } else if ext == "js" && dir_of(rel) == "js" {
        Kind::Fixed
    } else if ext == "js" {
        Kind::Js
    } else if ext == "html" {
        Kind::Html
    } else if ext == "xml" {
        Kind::Xml
    } else {
        Kind::Copy
    }
}

fn ext_of(rel: &str) -> &str {
    match rel.rfind('.') {
        Some(dot) => rel.split_at(dot + 1).1,
        None => "",
    }
}

fn dir_of(rel: &str) -> &str {
    match rel.rfind('/') {
        Some(slash) => rel.split_at(slash).0,
        None => "",
    }
}

fn utf8(rel: &str, bytes: &[u8]) -> Result<String, String> {
    std::str::from_utf8(bytes)
        .map(str::to_owned)
        .map_err(|_| format!("{rel} is not valid UTF-8"))
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

    /// A fresh scratch template dir under the temp dir.
    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("faro-site-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("the scratch dir can be made");
        dir
    }

    #[test]
    fn a_symlink_in_the_template_is_an_error() {
        let dir = scratch("symlink");
        fs::write(dir.join("index.html"), "<p>hi</p>").expect("index.html");
        std::os::unix::fs::symlink(dir.join("index.html"), dir.join("link.html"))
            .expect("the symlink can be made");
        let Err(problem) = generate(&dir, &dir.join("out"), None) else {
            panic!("a symlink should stop the build");
        };
        assert!(problem.contains("symlink"), "{problem}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_stylesheet_pointing_at_another_is_an_error() {
        let dir = scratch("two-css");
        fs::write(dir.join("index.html"), "<link href=\"style.css\">").expect("index.html");
        fs::write(dir.join("style.css"), "@import \"other.css\";").expect("style.css");
        fs::write(dir.join("other.css"), "body { color: red; }").expect("other.css");
        let Err(problem) = generate(&dir, &dir.join("out"), None) else {
            panic!("a second style sheet should stop the build");
        };
        assert!(problem.contains("cross-stylesheet"), "{problem}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn braces_in_a_js_file_are_an_error() {
        let dir = scratch("js-braces");
        fs::write(dir.join("index.html"), "<script src=\"app.js\"></script>").expect("index.html");
        fs::write(dir.join("app.js"), "const t = '{{version}}';").expect("app.js");
        let Err(problem) = generate(&dir, &dir.join("out"), None) else {
            panic!("braces in JS should stop the build");
        };
        assert!(problem.contains("JS files are copied"), "{problem}");
        let _ = fs::remove_dir_all(&dir);
    }

    /// wasm-bindgen's output, in miniature: the loader's fallback wasm name
    /// is the only thing `demo::copy` has to recognise.
    fn fake_pkg(dir: &Path) {
        fs::create_dir_all(dir).expect("the fake pkg dir");
        fs::write(
            dir.join("faro_web_bg.wasm"),
            b"wasm bytes, quite long to compress",
        )
        .expect("wasm");
        fs::write(
            dir.join("faro_web.js"),
            b"module_or_path = new URL('faro_web_bg.wasm', import.meta.url);",
        )
        .expect("js");
    }

    /// The real template, built end to end: the same thing `site/build.sh`
    /// does, minus the precompression.
    #[test]
    fn builds_the_real_template() {
        let template = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../site/template");
        let out = std::env::temp_dir().join(format!("faro-site-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&out);
        let pkg = scratch("pkg");
        fake_pkg(&pkg);
        generate(&template, &out, Some(&pkg)).expect("the real template builds");

        let mut files: Vec<(String, Vec<u8>)> = Vec::new();
        walk(&out, "", &mut files).expect("the output can be read back");
        let names: Vec<&str> = files.iter().map(|(rel, _)| rel.as_str()).collect();

        // Placeholders are gone from every text file, and the facts line is
        // filled. (Binary files are skipped: a PNG's bytes can contain "{{".)
        let text_exts = ["html", "css", "js", "svg", "json", "txt", "xml"];
        for (rel, bytes) in &files {
            if !text_exts.iter().any(|ext| has_ext(rel, ext)) {
                continue;
            }
            let text = String::from_utf8_lossy(bytes);
            assert!(!text.contains("{{"), "{rel} still has a placeholder");
        }
        let index = files
            .iter()
            .find(|(rel, _)| rel == "index.html")
            .map(|(_, bytes)| String::from_utf8_lossy(bytes).into_owned())
            .expect("index.html");
        assert!(index.contains(&format!("v{}", env!("CARGO_PKG_VERSION"))));
        assert!(index.contains("<kbd>Ctrl+P</kbd>"));
        // The poster and the OG image are rendered by the generator itself
        // (chunk 4a/4b): both schemes inline, the PNG hashed like any asset.
        assert!(
            index.contains(r#"class="poster-dark""#),
            "the dark poster is inline"
        );
        assert!(
            index.contains(r#"class="poster-light""#),
            "the light poster is inline"
        );
        assert!(
            index.contains("Scranton Branch"),
            "the poster shows the branch chat"
        );
        let og = names
            .iter()
            .find(|name| name.starts_with("og.") && has_ext(name, "png"))
            .expect("the hashed OG image");
        assert!(!names.contains(&"og.png"));
        assert!(index.contains(&format!("https://getfaro.dev/{og}")));

        // Hashed names replaced the plain ones, and the page points at them.
        assert!(!names.contains(&"style.css"));
        let css = names
            .iter()
            .find(|name| name.starts_with("style.") && has_ext(name, "css"))
            .expect("a hashed style sheet");
        assert!(index.contains(css));
        assert!(!names.contains(&"theme.js"));
        assert!(
            names
                .iter()
                .any(|name| name.starts_with("theme.") && has_ext(name, "js"))
        );
        let font = names
            .iter()
            .find(|name| name.starts_with("fonts/JetBrainsMono-Regular.") && has_ext(name, "woff2"))
            .expect("a hashed font");
        assert!(!names.contains(&"fonts/JetBrainsMono-Regular.woff2"));
        // The hashed CSS points at the hashed fonts.
        let css_bytes = files
            .iter()
            .find(|(rel, _)| rel == css)
            .map(|(_, bytes)| String::from_utf8_lossy(bytes).into_owned())
            .expect("the style sheet's bytes");
        assert!(css_bytes.contains(&format!("url(\"{font}\")")));

        // Fixed names under the proxied path survive untouched.
        assert!(names.contains(&"js/init.js"));
        assert!(names.contains(&"fonts/OFL.txt"));
        assert!(names.contains(&"privacy.html"));

        // Phase 5: the 404 page, robots and the sitemap keep their plain
        // names (crawlers and browsers fetch those URLs), and the sitemap's
        // lastmod is filled.
        assert!(names.contains(&"404.html"));
        assert!(names.contains(&"robots.txt"));
        assert!(names.contains(&"sitemap.xml"));
        let sitemap = files
            .iter()
            .find(|(rel, _)| rel == "sitemap.xml")
            .map(|(_, bytes)| String::from_utf8_lossy(bytes).into_owned())
            .expect("sitemap.xml");
        assert!(
            sitemap.contains(&format!(
                "<lastmod>{}</lastmod>",
                release::date().expect("a date")
            )),
            "the sitemap's lastmod is the build date"
        );
        // The 404 page's absolute asset references are hashed like any others.
        let page_404 = files
            .iter()
            .find(|(rel, _)| rel == "404.html")
            .map(|(_, bytes)| String::from_utf8_lossy(bytes).into_owned())
            .expect("404.html");
        assert!(
            page_404.contains(&format!("href=\"/{css}\"")),
            "the 404 page points at the hashed style sheet"
        );

        // The web demo ships beside the page (Phase 4).
        the_demo_ships_beside_the_page(&names, &files, &index);

        // The phone gallery (4.5b): the markup is in the page, and the six
        // frames are hashed like every other asset, pulled in by the
        // style sheet's url()s.
        assert!(
            index.contains(r#"class="phone-gallery""#),
            "the phone gallery is in the page"
        );
        the_phone_frames_are_hashed_into_the_style_sheet(&names, &css_bytes);

        let _ = fs::remove_dir_all(&out);
        let _ = fs::remove_dir_all(&pkg);
    }

    /// Every phone frame ships under a content-hashed name — never the
    /// plain one — and the hashed style sheet points at each hashed file
    /// (the gallery's backgrounds, §4.7).
    fn the_phone_frames_are_hashed_into_the_style_sheet(names: &[&str], css: &str) {
        for kind in ["conversation", "palette", "themes"] {
            for scheme in ["dark", "light"] {
                let plain = format!("phone/{kind}-{scheme}.svg");
                assert!(!names.contains(&plain.as_str()), "{plain} stays unhashed");
                let stem = format!("phone/{kind}-{scheme}.");
                let found = names
                    .iter()
                    .find(|name| name.starts_with(stem.as_str()) && has_ext(name, "svg"))
                    .unwrap_or_else(|| panic!("{kind}-{scheme}: a hashed phone frame is shipped"));
                let hash = &found[stem.len()..found.len() - ".svg".len()];
                assert!(
                    hash.len() == 10 && hash.bytes().all(|b| b.is_ascii_hexdigit()),
                    "{found} isn't content-hashed"
                );
                assert!(
                    css.contains(&format!(r#"url("{found}")"#)),
                    "the style sheet points at {found}"
                );
            }
        }
    }

    /// The demo's files are in the build, hashed like any other asset, both
    /// manifests are there, and the landing page reaches them only through
    /// its own hashed loader (chunk 2): the loader fetches the manifest, so
    /// the page itself names no demo file.
    fn the_demo_ships_beside_the_page(names: &[&str], files: &[(String, Vec<u8>)], index: &str) {
        let manifest = files
            .iter()
            .find(|(rel, _)| rel == "demo/manifest.json")
            .map(|(_, bytes)| String::from_utf8_lossy(bytes).into_owned())
            .expect("the demo manifest");
        assert!(manifest.contains("\"preload\":true"), "{manifest}");
        let no_preload = files
            .iter()
            .find(|(rel, _)| rel == "demo/manifest.no-preload.json")
            .map(|(_, bytes)| String::from_utf8_lossy(bytes).into_owned())
            .expect("the demo's no-preload manifest (nginx's emergency switch, §6.8)");
        assert!(no_preload.contains("\"preload\":false"), "{no_preload}");
        assert!(
            names
                .iter()
                .any(|name| name.starts_with("demo/faro_web.") && has_ext(name, "js")),
            "{names:?}"
        );
        assert!(
            names
                .iter()
                .any(|name| name.starts_with("demo/faro_web_bg.") && has_ext(name, "wasm")),
            "{names:?}"
        );
        assert!(
            !names.contains(&"demo/faro_web.js"),
            "the demo's plain names are gone"
        );
        assert!(
            !names.contains(&"demo.js"),
            "the loader's plain name is gone too"
        );
        let loader = names
            .iter()
            .find(|name| name.starts_with("demo.") && has_ext(name, "js"))
            .expect("the hashed demo loader");
        assert!(index.contains(loader), "the page loads the demo (D124)");
        assert!(
            !index.contains("/demo/"),
            "the page names no demo file itself: demo.js fetches the manifest (§4.2)"
        );
    }

    #[test]
    fn kinds_sort_the_template_out() {
        assert_eq!(kind("style.css"), Kind::Css);
        assert_eq!(kind("theme.js"), Kind::Js);
        assert_eq!(kind("js/init.js"), Kind::Fixed);
        assert_eq!(kind("index.html"), Kind::Html);
        assert_eq!(kind("sitemap.xml"), Kind::Xml);
        assert_eq!(kind("fonts/x.woff2"), Kind::Font);
        assert_eq!(kind("fonts/OFL.txt"), Kind::Copy);
        assert_eq!(kind("robots.txt"), Kind::Copy);
    }
}
