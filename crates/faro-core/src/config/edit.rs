//! Writing one setting back to the config file's text, keeping everything
//! else (comments, order, spacing) as it was (DESIGN.md §9, D48).

use toml_edit::{DocumentMut, Item};

use super::{ConfigValue, split, syntax_error};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EditError {
    /// The file isn't valid TOML; it's left alone.
    #[error("the config file has an error ({0})")]
    Syntax(String),
    /// Something else sits where the setting's table should be.
    #[error("{0} in the config file isn't a table")]
    NotATable(String),
}

/// `text` with `path` (`table.key` or a top-level `key`) set to `value`.
///
/// If the key is set, only its value changes. If it's there but commented
/// out (as in the template), that line is uncommented and gets the value.
/// Otherwise the key is added to its table, which is created if needed.
pub fn set(text: &str, path: &str, value: &ConfigValue) -> Result<String, EditError> {
    let mut doc = text
        .parse::<DocumentMut>()
        .map_err(|error| EditError::Syntax(syntax_error(text, &error)))?;
    let (table, key) = split(path);
    if let Some(current) = find_mut(&mut doc, table, key) {
        if value.matches(current) {
            return Ok(text.to_owned());
        }
        let decor = current.decor().clone();
        *current = value.to_toml();
        *current.decor_mut() = decor;
        return Ok(doc.to_string());
    }
    let has_table = table.is_empty() || doc.contains_key(table);
    let attempts = [
        uncomment(text, table, key, value),
        // toml_edit would put these before the file's trailing comments, or
        // a top-level key after them; as text, where it's obvious.
        (!has_table).then(|| append_table(text, table, key, value)),
        table.is_empty().then(|| insert_top_level(text, key, value)),
    ];
    for edited in attempts.into_iter().flatten() {
        if let Ok(mut check) = edited.parse::<DocumentMut>()
            && find_mut(&mut check, table, key).is_some_and(|v| value.matches(v))
        {
            return Ok(edited);
        }
    }
    insert(&mut doc, table, key, value)?;
    Ok(doc.to_string())
}

/// `text` plus a new `[table]` holding the key, at the end.
fn append_table(text: &str, table: &str, key: &str, value: &ConfigValue) -> String {
    let mut out = text.to_owned();
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    if !out.is_empty() {
        out.push('\n');
    }
    out = format!("{out}[{table}]\n{key} = {value}\n");
    out
}

/// `text` with `key = value` added before the first table (and the comments
/// right above it, which belong to that table), or at the end.
fn insert_top_level(text: &str, key: &str, value: &ConfigValue) -> String {
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let Some(header) = lines
        .iter()
        .position(|l| table_header(l.trim_start()).is_some())
    else {
        let mut out = text.to_owned();
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        out = format!("{out}{key} = {value}\n");
        return out;
    };
    let mut at = header;
    while at > 0
        && lines
            .get(at - 1)
            .is_some_and(|l| l.trim_start().starts_with('#'))
    {
        at -= 1;
    }
    let before: String = lines.get(..at).unwrap_or_default().concat();
    let after: String = lines.get(at..).unwrap_or_default().concat();
    let gap = if before.is_empty() || before.ends_with("\n\n") {
        ""
    } else {
        "\n"
    };
    format!("{before}{gap}{key} = {value}\n\n{after}")
}

fn find_mut<'a>(
    doc: &'a mut DocumentMut,
    table: &str,
    key: &str,
) -> Option<&'a mut toml_edit::Value> {
    let item = if table.is_empty() {
        doc.get_mut(key)
    } else {
        doc.get_mut(table)
            .and_then(Item::as_table_like_mut)
            .and_then(|t| t.get_mut(key))
    };
    item.and_then(Item::as_value_mut)
}

fn insert(
    doc: &mut DocumentMut,
    table: &str,
    key: &str,
    value: &ConfigValue,
) -> Result<(), EditError> {
    let item = toml_edit::value(value.to_toml());
    if table.is_empty() {
        doc.insert(key, item);
        return Ok(());
    }
    let tables = doc
        .entry(table)
        .or_insert_with(toml_edit::table)
        .as_table_like_mut()
        .ok_or_else(|| EditError::NotATable(table.to_owned()))?;
    tables.insert(key, item);
    Ok(())
}

/// Replace the first `# key = …` line inside `table` with `key = value`.
fn uncomment(text: &str, table: &str, key: &str, value: &ConfigValue) -> Option<String> {
    let mut current = "";
    let mut out = String::with_capacity(text.len() + 8);
    let mut done = false;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_start();
        if let Some(header) = table_header(trimmed) {
            current = header;
        } else if !done && current == table && commented_key(trimmed, key) {
            let indent = line.get(..line.len() - trimmed.len()).unwrap_or_default();
            let ending = if line.ends_with("\r\n") {
                "\r\n"
            } else if line.ends_with('\n') {
                "\n"
            } else {
                ""
            };
            out.extend([indent, key, " = ", &value.to_string(), ending]);
            done = true;
            continue;
        }
        out.push_str(line);
    }
    done.then_some(out)
}

/// The name in a `[table]` header line (not `[[array]]`).
fn table_header(line: &str) -> Option<&str> {
    let rest = line.strip_prefix('[')?;
    if rest.starts_with('[') {
        return Some("[[");
    }
    rest.split_once(']').map(|(name, _)| name.trim())
}

/// `# key = …`, with any spacing.
fn commented_key(line: &str, key: &str) -> bool {
    line.strip_prefix('#')
        .map(str::trim_start)
        .and_then(|rest| rest.strip_prefix(key))
        .is_some_and(|rest| rest.trim_start().starts_with('='))
}
