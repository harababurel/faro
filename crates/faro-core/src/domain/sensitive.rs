//! Data that must not reach logs by accident (DESIGN.md §12, DECISIONS D29).
//!
//! These types have a redacting `Debug` and no `Display`, so `tracing::info!(?body)`,
//! `{:?}` and `#[derive(Debug)]` on containing types all print `<redacted>`.
//! Reading the value needs an explicit, greppable `.expose()`.

use std::fmt;

macro_rules! sensitive_string {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Clone, PartialEq, Eq, Hash)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            /// The real value. Never pass it to logging.
            pub fn expose(&self) -> &str {
                &self.0
            }

            pub fn is_empty(&self) -> bool {
                self.0.is_empty()
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(concat!(stringify!($name), "(<redacted>)"))
            }
        }
    };
}

sensitive_string!(
    /// The text of a message (or a quote's preview, or an edit).
    MessageBody
);

sensitive_string!(
    /// A phone number, as Signal provides it (usually E.164). Display data, never a key.
    PhoneNumber
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_never_shows_the_value() {
        let body = MessageBody::new("meet at 7, door code 4321");
        let phone = PhoneNumber::new("+15555550123");
        assert_eq!(format!("{body:?}"), "MessageBody(<redacted>)");
        assert_eq!(format!("{phone:?}"), "PhoneNumber(<redacted>)");

        // Also when nested in derived Debug output.
        let nested = format!("{:?}", Some((&body, vec![phone.clone()])));
        assert!(!nested.contains("4321") && !nested.contains("555"));

        assert_eq!(body.expose(), "meet at 7, door code 4321");
        assert_eq!(phone.expose(), "+15555550123");
    }
}
