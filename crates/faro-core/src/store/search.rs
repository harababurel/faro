//! Search query parser for [`Store::search_messages`] (`from:`, `in:`, `has:`,
//! `is:`, `before:`, `after:` + free-text body / transcript / filename tokens).

/// Parsed representation of a search query string.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SearchQuery {
    /// Lowercased free-text tokens matched against `body`, `transcript`, or `file_name`.
    pub terms: Vec<String>,
    /// Lowercased `from:<who>` filters.
    pub from: Vec<String>,
    /// Lowercased `in:<chat>` filters.
    pub in_chat: Vec<String>,
    /// `has:<kind>` filters.
    pub has: Vec<HasFilter>,
    /// `is:<state>` filters.
    pub is: Vec<IsFilter>,
    /// Exclusive upper bound (`sent_at < before_ms`) from `before:<date>`.
    pub before_ms: Option<i64>,
    /// Inclusive lower bound (`sent_at >= after_ms`) from `after:<date>` / `since:<date>`.
    pub after_ms: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HasFilter {
    Image,
    Video,
    Voice,
    File,
    Link,
    Transcript,
    Reaction,
    Reply,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IsFilter {
    Unread,
    Read,
    Edited,
    Incoming,
    Outgoing,
}

impl SearchQuery {
    #[must_use]
    pub fn parse(raw: &str) -> Self {
        let mut out = Self::default();
        for raw_token in raw.split_whitespace() {
            let token = raw_token.to_lowercase();
            if let Some((key, value)) = token.split_once(':')
                && !value.is_empty()
                && out.apply_operator(key, value)
            {
                continue;
            }
            out.terms.push(token);
        }
        out
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.terms.is_empty()
            && self.from.is_empty()
            && self.in_chat.is_empty()
            && self.has.is_empty()
            && self.is.is_empty()
            && self.before_ms.is_none()
            && self.after_ms.is_none()
    }

    fn apply_operator(&mut self, key: &str, value: &str) -> bool {
        match key {
            "from" => {
                self.from.push(value.to_owned());
                true
            }
            "in" | "chat" => {
                self.in_chat.push(value.to_owned());
                true
            }
            "has" => {
                let Some(filter) = HasFilter::parse(value) else {
                    return false;
                };
                self.has.push(filter);
                true
            }
            "is" => {
                let Some(filter) = IsFilter::parse(value) else {
                    return false;
                };
                self.is.push(filter);
                true
            }
            "before" => {
                let Some(ms) = parse_date_start_ms(value) else {
                    return false;
                };
                self.before_ms = Some(self.before_ms.map_or(ms, |old| old.min(ms)));
                true
            }
            "after" | "since" => {
                let Some(ms) = parse_date_start_ms(value) else {
                    return false;
                };
                self.after_ms = Some(self.after_ms.map_or(ms, |old| old.max(ms)));
                true
            }
            _ => false,
        }
    }
}

impl HasFilter {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "image" | "images" | "photo" | "photos" | "pic" | "pics" => Some(Self::Image),
            "video" | "videos" => Some(Self::Video),
            "voice" | "audio" | "voicenote" => Some(Self::Voice),
            "file" | "files" | "attachment" | "attachments" | "media" => Some(Self::File),
            "link" | "links" | "url" | "urls" => Some(Self::Link),
            "transcript" | "transcripts" => Some(Self::Transcript),
            "reaction" | "reactions" => Some(Self::Reaction),
            "reply" | "quote" => Some(Self::Reply),
            _ => None,
        }
    }
}

impl IsFilter {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "unread" => Some(Self::Unread),
            "read" => Some(Self::Read),
            "edited" => Some(Self::Edited),
            "incoming" => Some(Self::Incoming),
            "outgoing" | "sent" => Some(Self::Outgoing),
            _ => None,
        }
    }
}

/// Parse `YYYY`, `YYYY-MM`, or `YYYY-MM-DD` into the UTC start-of-period
/// timestamp in milliseconds since the Unix epoch.
#[must_use]
pub fn parse_date_start_ms(value: &str) -> Option<i64> {
    let parts: Vec<&str> = value.split('-').collect();
    let (year, month, day): (i64, u32, u32) = match parts.as_slice() {
        [y] if y.len() == 4 => (y.parse().ok()?, 1, 1),
        [y, m] if y.len() == 4 && (1..=2).contains(&m.len()) => (y.parse().ok()?, m.parse().ok()?, 1),
        [y, m, d]
            if y.len() == 4 && (1..=2).contains(&m.len()) && (1..=2).contains(&d.len()) =>
        {
            (y.parse().ok()?, m.parse().ok()?, d.parse().ok()?)
        }
        _ => return None,
    };
    if !(1970..=9999).contains(&year) || !(1..=12).contains(&month) {
        return None;
    }
    let max_day = days_in_month(year, month);
    if !(1..=max_day).contains(&day) {
        return None;
    }
    let days = days_from_civil(year, i64::from(month), i64::from(day));
    days.checked_mul(86_400_000)
}

const fn is_leap_year(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

const fn days_in_month(year: i64, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

/// Howard Hinnant's civil-date-to-days-since-1970-01-01 algorithm.
const fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = if month > 2 { month - 3 } else { month + 9 };
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}
