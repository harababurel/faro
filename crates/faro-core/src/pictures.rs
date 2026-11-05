//! Conversations' pictures on disk (D160): one file per conversation in the
//! data directory's `pictures/` folder, named after its ID (`direct:<id>` or
//! `group:<hex>`, with `_` for `:`). `<name>.pic` holds the picture as
//! Signal sent it (JPEG or PNG; readers guess the format); an empty
//! `<name>.none` records that the conversation has none, so it isn't asked
//! for again. Both stay until `faro refresh-pictures` empties the folder.

use crate::domain::ConversationId;

/// A picture's extension.
pub const PICTURE: &str = "pic";
/// A "has no picture" mark's extension.
pub const NONE: &str = "none";

/// The file for `id`: its picture, or the mark that it has none.
pub fn file_name(id: ConversationId, has_picture: bool) -> String {
    let stem = id.to_string().replace(':', "_");
    let extension = if has_picture { PICTURE } else { NONE };
    format!("{stem}.{extension}")
}

/// Whose file `name` is, and whether it's a picture (`true`) or a mark.
/// `None` for anything else in the folder (temporary files, strays).
pub fn parse(name: &str) -> Option<(ConversationId, bool)> {
    let (stem, extension) = name.rsplit_once('.')?;
    let has_picture = match extension {
        PICTURE => true,
        NONE => false,
        _ => return None,
    };
    let id = stem.replace('_', ":").parse().ok()?;
    Some((id, has_picture))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{GroupId, ServiceId};

    #[test]
    fn names_round_trip_and_strays_are_ignored() {
        let person = ConversationId::Direct(ServiceId::Aci(uuid::Uuid::from_u128(7)));
        let pni = ConversationId::Direct(ServiceId::Pni(uuid::Uuid::from_u128(8)));
        let group = ConversationId::Group("ab".repeat(32).parse::<GroupId>().unwrap());
        for id in [person, pni, group] {
            for has in [true, false] {
                let name = file_name(id, has);
                assert!(!name.contains(':') && !name.contains('/'), "{name}");
                assert_eq!(parse(&name), Some((id, has)), "{name}");
            }
        }
        assert_eq!(
            file_name(person, true),
            "direct_00000000-0000-0000-0000-000000000007.pic"
        );
        assert_eq!(parse("direct_x.pic"), None);
        assert_eq!(parse(".tmp-123"), None);
        assert_eq!(parse("notes.txt"), None);
    }
}
