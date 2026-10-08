//! Theme files in OpenCode's JSON format (D54): parsed to plain RGBA colours
//! here (no terminal types; `crates/faro` turns them into a `Theme`).
//!
//! ```json
//! { "$schema": "…", "defs": { "nord0": "#2e3440" },
//!   "theme": { "background": "nord0", "text": { "dark": "#eceff4", "light": "nord0" } } }
//! ```
//!
//! A colour is `"#rgb"`, `"#rgba"`, `"#rrggbb"` or `"#rrggbbaa"`; the name of
//! a `defs` entry or of another role; `"transparent"`/`"none"`; an ANSI
//! colour number (0–255, with OpenCode's values for 0–15); or
//! `{ "dark": …, "light": … }` for the two variants. References are followed
//! (defs first, then roles); a cycle is an error.

use std::collections::BTreeMap;

use serde_json::Value;

/// A theme's dark or light variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Dark,
    Light,
}

impl Mode {
    const fn key(self) -> &'static str {
        match self {
            Self::Dark => "dark",
            Self::Light => "light",
        }
    }
}

/// A colour with opacity (`a` = 255: opaque; 0: transparent).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba {
    pub const TRANSPARENT: Self = Self {
        r: 0,
        g: 0,
        b: 0,
        a: 0,
    };

    pub const fn opaque(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }
}

/// What's wrong with a theme file.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ThemeError {
    #[error("not valid JSON: {0}")]
    Json(String),
    #[error("there's no \"theme\" object")]
    NoTheme,
    #[error("{role}: {problem}")]
    Role { role: String, problem: String },
}

/// A parsed theme file; colours are resolved per role and variant.
#[derive(Debug, Clone, PartialEq)]
pub struct ThemeFile {
    defs: BTreeMap<String, Value>,
    theme: BTreeMap<String, Value>,
}

/// Read a theme file's text.
pub fn parse(text: &str) -> Result<ThemeFile, ThemeError> {
    let value: Value = serde_json::from_str(text).map_err(|e| ThemeError::Json(e.to_string()))?;
    let Value::Object(root) = value else {
        return Err(ThemeError::NoTheme);
    };
    let Some(Value::Object(theme)) = root.get("theme") else {
        return Err(ThemeError::NoTheme);
    };
    let defs = match root.get("defs") {
        Some(Value::Object(defs)) => defs.clone().into_iter().collect(),
        Some(_) | None => BTreeMap::new(),
    };
    Ok(ThemeFile {
        defs,
        theme: theme.clone().into_iter().collect(),
    })
}

impl ThemeFile {
    /// The colour of `role` in `mode`; `None` if the file doesn't set it.
    pub fn colour(&self, role: &str, mode: Mode) -> Result<Option<Rgba>, ThemeError> {
        let Some(value) = self.theme.get(role) else {
            return Ok(None);
        };
        // As OpenCode does: a role may name a def of its own name.
        let mut chain = Vec::new();
        self.resolve(value, mode, &mut chain)
            .map(Some)
            .map_err(|problem| ThemeError::Role {
                role: role.to_owned(),
                problem,
            })
    }

    fn resolve(&self, value: &Value, mode: Mode, chain: &mut Vec<String>) -> Result<Rgba, String> {
        match value {
            Value::String(text) => self.resolve_text(text, mode, chain),
            Value::Number(number) => number
                .as_u64()
                .and_then(|n| u8::try_from(n).ok())
                .map(ansi)
                .ok_or_else(|| format!("{number} isn't an ANSI colour (0–255)")),
            Value::Object(variants) => {
                let value = variants
                    .get(mode.key())
                    .ok_or_else(|| format!("no \"{}\" colour", mode.key()))?;
                self.resolve(value, mode, chain)
            }
            Value::Null | Value::Bool(_) | Value::Array(_) => {
                Err(format!("{value} isn't a colour"))
            }
        }
    }

    fn resolve_text(
        &self,
        text: &str,
        mode: Mode,
        chain: &mut Vec<String>,
    ) -> Result<Rgba, String> {
        if text == "transparent" || text == "none" {
            return Ok(Rgba::TRANSPARENT);
        }
        if let Some(hex) = text.strip_prefix('#') {
            return parse_hex(hex).ok_or_else(|| format!("\"{text}\" isn't a hex colour"));
        }
        if chain.iter().any(|seen| seen == text) {
            chain.push(text.to_owned());
            return Err(format!("circular reference ({})", chain.join(" → ")));
        }
        let next = self
            .defs
            .get(text)
            .or_else(|| self.theme.get(text))
            .ok_or_else(|| format!("\"{text}\" isn't defined"))?;
        chain.push(text.to_owned());
        let resolved = self.resolve(next, mode, chain);
        chain.pop();
        resolved
    }
}

/// `rgb`, `rgba`, `rrggbb` or `rrggbbaa`.
fn parse_hex(hex: &str) -> Option<Rgba> {
    if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let digit = |i: usize| {
        hex.get(i..=i)
            .and_then(|d| u8::from_str_radix(d, 16).ok())
            .map(|d| d * 17)
    };
    let pair = |i: usize| {
        hex.get(i..i + 2)
            .and_then(|p| u8::from_str_radix(p, 16).ok())
    };
    match hex.len() {
        3 => Some(Rgba::opaque(digit(0)?, digit(1)?, digit(2)?)),
        4 => Some(Rgba {
            a: digit(3)?,
            ..Rgba::opaque(digit(0)?, digit(1)?, digit(2)?)
        }),
        6 => Some(Rgba::opaque(pair(0)?, pair(2)?, pair(4)?)),
        8 => Some(Rgba {
            a: pair(6)?,
            ..Rgba::opaque(pair(0)?, pair(2)?, pair(4)?)
        }),
        _ => None,
    }
}

/// ANSI colour `code` as OpenCode draws it: its own values for 0–15, then
/// the xterm cube and grey ramp.
fn ansi(code: u8) -> Rgba {
    const BASIC: [(u8, u8, u8); 16] = [
        (0x00, 0x00, 0x00),
        (0x80, 0x00, 0x00),
        (0x00, 0x80, 0x00),
        (0x80, 0x80, 0x00),
        (0x00, 0x00, 0x80),
        (0x80, 0x00, 0x80),
        (0x00, 0x80, 0x80),
        (0xc0, 0xc0, 0xc0),
        (0x80, 0x80, 0x80),
        (0xff, 0x00, 0x00),
        (0x00, 0xff, 0x00),
        (0xff, 0xff, 0x00),
        (0x00, 0x00, 0xff),
        (0xff, 0x00, 0xff),
        (0x00, 0xff, 0xff),
        (0xff, 0xff, 0xff),
    ];
    let level = |x: u8| if x == 0 { 0 } else { x * 40 + 55 };
    match code {
        0..=15 => {
            let (r, g, b) = BASIC.get(usize::from(code)).copied().unwrap_or_default();
            Rgba::opaque(r, g, b)
        }
        16..=231 => {
            let i = code - 16;
            Rgba::opaque(level(i / 36), level(i / 6 % 6), level(i % 6))
        }
        232..=255 => {
            let grey = (code - 232) * 10 + 8;
            Rgba::opaque(grey, grey, grey)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(theme: &str) -> ThemeFile {
        parse(&format!(
            r##"{{ "$schema": "x", "defs": {{ "ink": "#112233", "loop": "loop2", "loop2": "loop", "m": "#010203" }},
                 "theme": {theme} }}"##
        ))
        .unwrap()
    }

    fn colour(file: &ThemeFile, role: &str, mode: Mode) -> Result<Option<Rgba>, ThemeError> {
        file.colour(role, mode)
    }

    #[test]
    fn every_colour_form() {
        let f = file(
            r##"{ "a": "#abc", "b": "#abcd", "c": "#a1b2c3", "d": "#a1b2c380",
                  "e": "ink", "f": "e", "g": { "dark": "ink", "light": "#fff" },
                  "h": "transparent", "i": "none", "j": 1, "k": 196, "l": 244,
                  "m": { "dark": "m" } }"##,
        );
        let dark = |role| colour(&f, role, Mode::Dark).unwrap().unwrap();
        assert_eq!(dark("a"), Rgba::opaque(0xaa, 0xbb, 0xcc));
        assert_eq!(
            dark("b"),
            Rgba {
                a: 0xdd,
                ..Rgba::opaque(0xaa, 0xbb, 0xcc)
            }
        );
        assert_eq!(dark("c"), Rgba::opaque(0xa1, 0xb2, 0xc3));
        assert_eq!(dark("d").a, 0x80);
        assert_eq!(dark("e"), Rgba::opaque(0x11, 0x22, 0x33));
        assert_eq!(dark("f"), dark("e"), "a role naming another role");
        assert_eq!(dark("g"), dark("e"));
        assert_eq!(
            colour(&f, "g", Mode::Light).unwrap(),
            Some(Rgba::opaque(0xff, 0xff, 0xff))
        );
        assert_eq!(dark("h"), Rgba::TRANSPARENT);
        assert_eq!(dark("i"), Rgba::TRANSPARENT);
        assert_eq!(dark("j"), Rgba::opaque(0x80, 0, 0));
        assert_eq!(dark("k"), Rgba::opaque(255, 0, 0));
        assert_eq!(dark("l"), Rgba::opaque(128, 128, 128));
        assert_eq!(
            dark("m"),
            Rgba::opaque(1, 2, 3),
            "a role naming a def of its name"
        );
        assert_eq!(colour(&f, "missing", Mode::Dark).unwrap(), None);
    }

    #[test]
    fn mistakes_name_the_role() {
        let f = file(
            r##"{ "a": "#12", "b": "nowhere", "c": "loop", "d": { "dark": "#fff" },
                  "e": 300, "f": true, "g": "#ggg" }"##,
        );
        let problem = |role, mode| match colour(&f, role, mode) {
            Err(ThemeError::Role { role, problem }) => format!("{role}: {problem}"),
            other => panic!("{other:?}"),
        };
        assert_eq!(problem("a", Mode::Dark), "a: \"#12\" isn't a hex colour");
        assert_eq!(problem("b", Mode::Dark), "b: \"nowhere\" isn't defined");
        assert!(
            problem("c", Mode::Dark).contains("circular reference (loop → loop2 → loop)"),
            "{}",
            problem("c", Mode::Dark)
        );
        assert_eq!(problem("d", Mode::Light), "d: no \"light\" colour");
        assert!(problem("e", Mode::Dark).contains("isn't an ANSI colour"));
        assert_eq!(problem("f", Mode::Dark), "f: true isn't a colour");
        assert_eq!(problem("g", Mode::Dark), "g: \"#ggg\" isn't a hex colour");
    }

    #[test]
    fn broken_files() {
        assert!(matches!(parse("{"), Err(ThemeError::Json(_))));
        assert_eq!(parse("[]"), Err(ThemeError::NoTheme));
        assert_eq!(parse(r#"{"defs": {}}"#), Err(ThemeError::NoTheme));
        assert_eq!(parse(r#"{"theme": 3}"#), Err(ThemeError::NoTheme));
        // No defs is fine.
        let f = parse(r##"{"theme": {"text": "#fff"}}"##).unwrap();
        assert_eq!(
            f.colour("text", Mode::Dark).unwrap(),
            Some(Rgba::opaque(255, 255, 255))
        );
    }
}
