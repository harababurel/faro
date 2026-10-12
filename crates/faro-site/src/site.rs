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
use crate::keys;
use crate::placeholders::{self, Problem, Syntax, Values};
use crate::poster;
use crate::release;
use crate::themes;

/// The OG image URL when `generated/og.png` doesn't exist yet (§5.4): the
/// meta tag must never be empty, and chunk 4 will put a real `og.png` at
/// that path.
const OG_FALLBACK_URL: &str = "https://getfaro.dev/og.png";

/// Font extensions: these get content-hashed names like CSS and JS do.
const FONT_EXTS: [&str; 4] = ["woff", "woff2", "ttf", "otf"];

/// The OG image as `(url to put in the meta tag, file to copy if any)`.
type Og = (String, Option<(String, Vec<u8>)>);

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
    Copy,
}

/// Build the site: copy the template, fill the placeholders from faro's
/// own code, and rename the assets to content-hashed names.
pub fn generate(template: &Path, out: &Path) -> Result<(), String> {
    let mut found: Vec<(String, Vec<u8>)> = Vec::new();
    walk(template, "", &mut found)?;
    let contents: BTreeMap<String, Vec<u8>> = found.into_iter().collect();

    let generated = template.join("..").join("generated");
    let (og_url, og_copy) = og_image(&generated)?;
    let poster = poster_text(&generated)?;
    let values = values(poster, og_url)?;

    // Hashed assets, in dependency order: fonts (content unchanged), then
    // CSS (hashed after its font URLs are rewritten), then JS.
    let mut hashed: BTreeMap<String, String> = BTreeMap::new();
    let mut written: BTreeMap<String, Vec<u8>> = BTreeMap::new();
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
        if matches!(kind(rel), Kind::Fixed | Kind::Copy) {
            written.insert(rel.clone(), bytes.clone());
        }
    }
    if let Some((name, bytes)) = og_copy {
        written.insert(name, bytes);
    }

    for (rel, bytes) in &written {
        let path = out.join(rel);
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).map_err(|error| io_err("create", dir, &error))?;
        }
        fs::write(&path, bytes).map_err(|error| io_err("write", &path, &error))?;
    }
    Ok(())
}

/// Every value the template's placeholders can name.
fn values(poster: String, og_url: String) -> Result<Values, String> {
    let keymap = Keymap::default();
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

/// The poster: chunk 3's `generated/poster.svg` when it exists, the
/// built-in fallback otherwise.
fn poster_text(generated: &Path) -> Result<String, String> {
    if let Some(bytes) = read_optional(&generated.join("poster.svg"))? {
        utf8("generated/poster.svg", &bytes)
    } else {
        Ok(poster::FALLBACK.to_owned())
    }
}

/// The OG image: hashed and copied when `generated/og.png` exists, plain
/// URL and no copy otherwise (§5.4).
fn og_image(generated: &Path) -> Result<Og, String> {
    if let Some(bytes) = read_optional(&generated.join("og.png"))? {
        let name = assets::hashed_name("og.png", &assets::hash_prefix(&bytes));
        Ok((format!("https://getfaro.dev/{name}"), Some((name, bytes))))
    } else {
        Ok((OG_FALLBACK_URL.to_owned(), None))
    }
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

fn read_optional(path: &Path) -> Result<Option<Vec<u8>>, String> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(io_err("read", path, &error)),
    }
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
        let Err(problem) = generate(&dir, &dir.join("out")) else {
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
        let Err(problem) = generate(&dir, &dir.join("out")) else {
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
        let Err(problem) = generate(&dir, &dir.join("out")) else {
            panic!("braces in JS should stop the build");
        };
        assert!(problem.contains("JS files are copied"), "{problem}");
        let _ = fs::remove_dir_all(&dir);
    }

    /// The real template, built end to end: the same thing `site/build.sh`
    /// does, minus the precompression.
    #[test]
    fn builds_the_real_template() {
        let template = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../site/template");
        let out = std::env::temp_dir().join(format!("faro-site-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&out);
        generate(&template, &out).expect("the real template builds");

        let mut files: Vec<(String, Vec<u8>)> = Vec::new();
        walk(&out, "", &mut files).expect("the output can be read back");
        let names: Vec<&str> = files.iter().map(|(rel, _)| rel.as_str()).collect();

        // Placeholders are gone everywhere, and the facts line is filled.
        for (rel, bytes) in &files {
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
        // The poster and the OG image fall back until chunk 3/4 writes
        // `site/generated/`; then the real ones must show up instead.
        if !template.join("../generated/poster.svg").exists() {
            assert!(index.contains("poster coming soon"));
        }
        if !template.join("../generated/og.png").exists() {
            assert!(index.contains(OG_FALLBACK_URL));
        }

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

        let _ = fs::remove_dir_all(&out);
    }

    #[test]
    fn kinds_sort_the_template_out() {
        assert_eq!(kind("style.css"), Kind::Css);
        assert_eq!(kind("theme.js"), Kind::Js);
        assert_eq!(kind("js/init.js"), Kind::Fixed);
        assert_eq!(kind("index.html"), Kind::Html);
        assert_eq!(kind("fonts/x.woff2"), Kind::Font);
        assert_eq!(kind("fonts/OFL.txt"), Kind::Copy);
    }
}
