//! Curated `:shortcode:` emoji table and search helper for inline input
//! completion and the searchable reaction picker.

/// Common everyday emoji and their `:shortcode:` names (lowercase ASCII).
/// Aliases for the same emoji (e.g. `"+1"` and `"thumbsup"`) may appear;
/// [`search_shortcodes`] deduplicates identical emoji in its results.
pub const SHORTCODES: &[(&str, &str)] = &[
    ("heart", "❤️"),
    ("+1", "👍"),
    ("thumbsup", "👍"),
    ("-1", "👎"),
    ("thumbsdown", "👎"),
    ("joy", "😂"),
    ("open_mouth", "😮"),
    ("cry", "😢"),
    ("fire", "🔥"),
    ("tada", "🎉"),
    ("rocket", "🚀"),
    ("eyes", "👀"),
    ("100", "💯"),
    ("sparkles", "✨"),
    ("clap", "👏"),
    ("pray", "🙏"),
    ("wave", "👋"),
    ("ok_hand", "👌"),
    ("muscle", "💪"),
    ("thinking", "🤔"),
    ("sunglasses", "😎"),
    ("sob", "😭"),
    ("sweat_smile", "😅"),
    ("rofl", "🤣"),
    ("wink", "😉"),
    ("blush", "😊"),
    ("heart_eyes", "😍"),
    ("partying", "🥳"),
    ("mind_blown", "🤯"),
    ("salute", "🫡"),
    ("melting", "🫠"),
    ("smile", "😄"),
    ("grin", "😁"),
    ("laughing", "😆"),
    ("kissing_heart", "😘"),
    ("sleeping", "😴"),
    ("scream", "😱"),
    ("angry", "😠"),
    ("rage", "😡"),
    ("skull", "💀"),
    ("poop", "💩"),
    ("clown", "🤡"),
    ("ghost", "👻"),
    ("alien", "👽"),
    ("robot", "🤖"),
    ("cat", "🐱"),
    ("dog", "🐶"),
    ("handshake", "🤝"),
    ("raised_hands", "🙌"),
    ("point_up", "☝️"),
    ("point_right", "👉"),
    ("point_left", "👈"),
    ("point_down", "👇"),
    ("broken_heart", "💔"),
    ("boom", "💥"),
    ("check", "✅"),
    ("x", "❌"),
    ("warning", "⚠️"),
    ("question", "❓"),
    ("exclamation", "❗"),
    ("star", "⭐"),
    ("zap", "⚡"),
    ("sun", "☀️"),
    ("moon", "🌙"),
    ("rainbow", "🌈"),
    ("coffee", "☕"),
    ("beer", "🍺"),
    ("pizza", "🍕"),
    ("cake", "🎂"),
    ("taco", "🌮"),
    ("bulb", "💡"),
    ("lock", "🔒"),
    ("key", "🔑"),
    ("bell", "🔔"),
    ("megaphone", "📣"),
    ("trophy", "🏆"),
    ("medal", "🏅"),
    ("gem", "💎"),
    ("moneybag", "💰"),
    ("chart_up", "📈"),
    ("bug", "🐛"),
    ("wrench", "🔧"),
    ("hammer", "🔨"),
    ("gear", "⚙️"),
    ("link", "🔗"),
    ("pin", "📌"),
    ("paperclip", "📎"),
    ("memo", "📝"),
    ("calendar", "📅"),
    ("clock", "🕒"),
    ("hourglass", "⏳"),
    ("mountain", "⛰️"),
    ("camping", "🏕️"),
    ("climbing", "🧗"),
    ("bike", "🚲"),
    ("car", "🚗"),
    ("airplane", "✈️"),
    ("ship", "🚢"),
    ("house", "🏠"),
];

/// Maximum number of `:shortcode:` candidates shown in the reaction search popup.
pub const REACT_SEARCH_LIMIT: usize = 8;

fn push_unique(
    results: &mut Vec<(&'static str, &'static str)>,
    limit: usize,
    name: &'static str,
    emoji: &'static str,
) -> bool {
    if results.len() < limit && !results.iter().any(|(_, e)| *e == emoji) {
        results.push((name, emoji));
    }
    results.len() >= limit
}

/// Match `query` (lowercased, stripped of leading/trailing `:`) against
/// [`SHORTCODES`]:
/// - If `query` is empty, returns up to `limit` entries in table order
///   (deduplicating identical emoji if aliases exist).
/// - Otherwise ranks prefix matches first, then substring matches, up to
///   `limit` (deduplicating identical emoji).
pub fn search_shortcodes(query: &str, limit: usize) -> Vec<(&'static str, &'static str)> {
    if limit == 0 {
        return Vec::new();
    }
    let cleaned = query.trim().trim_matches(':').to_ascii_lowercase();
    let mut results: Vec<(&'static str, &'static str)> = Vec::with_capacity(limit);
    if cleaned.is_empty() {
        for &(name, emoji) in SHORTCODES {
            if push_unique(&mut results, limit, name, emoji) {
                break;
            }
        }
        return results;
    }
    for &(name, emoji) in SHORTCODES {
        if name == cleaned && push_unique(&mut results, limit, name, emoji) {
            return results;
        }
    }
    for &(name, emoji) in SHORTCODES {
        if name.starts_with(&cleaned) && push_unique(&mut results, limit, name, emoji) {
            return results;
        }
    }
    for &(name, emoji) in SHORTCODES {
        if !name.starts_with(&cleaned)
            && name.contains(&cleaned)
            && push_unique(&mut results, limit, name, emoji)
        {
            return results;
        }
    }
    results
}

#[cfg(test)]
mod tests {
    use unicode_segmentation::UnicodeSegmentation as _;
    use unicode_width::{UnicodeWidthChar as _, UnicodeWidthStr as _};

    use super::*;
    use crate::app::state::EmojiMode;
    use crate::ui::{emoji, text};

    #[test]
    fn every_shortcode_is_valid_and_sanitises_cleanly_in_both_modes() {
        assert!(SHORTCODES.len() >= 80);
        for &(name, raw_emoji) in SHORTCODES {
            assert!(!name.is_empty(), "empty shortcode name");
            assert!(
                name.chars().all(|c| c.is_ascii_lowercase()
                    || c.is_ascii_digit()
                    || matches!(c, '_' | '+' | '-')),
                "shortcode {name:?} must be lowercase ASCII"
            );
            assert!(!raw_emoji.is_empty(), "empty emoji for {name}");

            emoji::set_mode(EmojiMode::Full);
            let full = text::sanitize(raw_emoji);
            assert_eq!(
                full, raw_emoji,
                "{name} should already be canonical in Full mode"
            );
            assert_eq!(
                text::width(&full),
                2,
                "{name} ({raw_emoji:?}) should be 2 cells wide in Full mode"
            );
            for g in full.graphemes(true) {
                assert!(
                    emoji::drawable(g),
                    "{name}: {g:?} not drawable in Full mode"
                );
            }

            emoji::set_mode(EmojiMode::Compatible);
            let compat = text::sanitize(raw_emoji);
            assert!(!compat.is_empty(), "{name}: empty in Compatible mode");
            let compat_width = text::width(&compat);
            assert!(
                (1..=2).contains(&compat_width),
                "{name}: unexpected Compatible width {compat_width}"
            );
            for g in compat.graphemes(true) {
                assert!(
                    emoji::drawable(g),
                    "{name}: {g:?} not drawable in Compatible mode"
                );
                let cp_sum: usize = g.chars().map(|c| c.width().unwrap_or(0)).sum();
                assert_eq!(
                    g.width(),
                    cp_sum,
                    "{name}: {g:?} grapheme width disagrees with codepoint sum"
                );
            }
        }
        emoji::set_mode(EmojiMode::Full);
    }

    #[test]
    fn search_shortcodes_ranks_prefix_before_substring_and_deduplicates() {
        let empty = search_shortcodes("", 5);
        assert_eq!(empty.len(), 5);
        // "+1" and "thumbsup" both map to 👍, so 👍 appears only once.
        assert_eq!(empty.iter().filter(|(_, e)| *e == "👍").count(), 1);

        let heart = search_shortcodes("heart", 5);
        assert_eq!(heart.first(), Some(&("heart", "❤️")));
        // Prefix ("heart", "heart_eyes") comes before substring ("kissing_heart", "broken_heart").
        let names: Vec<&str> = heart.iter().map(|(n, _)| *n).collect();
        assert_eq!(
            names,
            vec!["heart", "heart_eyes", "kissing_heart", "broken_heart"]
        );

        let fire = search_shortcodes(":fir:", 5);
        assert!(fire.contains(&("fire", "🔥")));
    }
}
