//! Stand-in pictures for the mock's people and groups (D160), drawn rather
//! than shipped: a head and shoulders on a soft gradient for a person, three
//! heads for a group, each in colours picked from its ID. People known only
//! by their number have none, so the demo shows initials too. Encoded as PNG
//! with stored (uncompressed) deflate blocks: no image crate in the core.

use super::world::World;
use crate::domain::ConversationId;

const SIZE: u32 = 96;

/// `id`'s picture, or `None` when it has none.
pub(super) fn picture(world: &World, id: ConversationId) -> Option<Vec<u8>> {
    let heads: &[(f32, f32, f32)] = match id {
        ConversationId::Direct(who) => {
            let contact = world.contacts.iter().find(|c| c.service_id == who)?;
            if contact.name.is_none() && contact.profile_name.is_none() {
                return None;
            }
            &[(0.5, 0.40, 0.19)]
        }
        ConversationId::Group(group) => {
            if !world.has_group(group) {
                return None;
            }
            &[(0.27, 0.46, 0.13), (0.73, 0.46, 0.13), (0.5, 0.36, 0.15)]
        }
    };
    let hue = hue_of(&id.to_string());
    Some(png(SIZE, SIZE, &draw(hue, heads)))
}

/// A hue in `0..1` from a name (FNV-1a), the same on every run.
fn hue_of(name: &str) -> f32 {
    let hash = name.bytes().fold(0x811c_9dc5_u32, |h, b| {
        (h ^ u32::from(b)).wrapping_mul(0x0100_0193)
    });
    f32::from(u16::try_from(hash % 1000).unwrap_or(0)) / 1000.0
}

/// RGB rows: a vertical gradient with each head (centre x, centre y,
/// radius, as fractions of the size) and its shoulders in a contrasting
/// colour.
#[expect(
    clippy::cast_precision_loss,
    reason = "pixel coordinates of a 96-pixel picture"
)]
fn draw(hue: f32, heads: &[(f32, f32, f32)]) -> Vec<u8> {
    let size = SIZE as f32;
    let top = hsv(hue, 0.45, 0.95);
    let bottom = hsv(hue + 0.1, 0.7, 0.6);
    let figure = hsv(hue + 0.5, 0.45, 0.35);
    let mut pixels = Vec::with_capacity((SIZE * SIZE * 3) as usize);
    for y in 0..SIZE {
        let t = y as f32 / (size - 1.0);
        let fy = y as f32 / size;
        for x in 0..SIZE {
            let fx = x as f32 / size;
            let inside = heads.iter().any(|&(cx, cy, r)| {
                let head = (fx - cx).hypot(fy - cy) < r;
                let (sx, sy) = ((fx - cx) / (r * 2.1), (fy - (cy + r * 2.4)) / (r * 1.6));
                head || sx * sx + sy * sy < 1.0
            });
            let colour = if inside {
                figure
            } else {
                let [r0, g0, b0] = top;
                let [r1, g1, b1] = bottom;
                [lerp(r0, r1, t), lerp(g0, g1, t), lerp(b0, b1, t)]
            };
            pixels.extend_from_slice(&colour);
        }
    }
    pixels
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "colour channels clamped to 0..=255"
)]
fn lerp(a: u8, b: u8, t: f32) -> u8 {
    (f32::from(a) + (f32::from(b) - f32::from(a)) * t)
        .round()
        .clamp(0.0, 255.0) as u8
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "colour channels clamped to 0..=255"
)]
fn hsv(hue: f32, saturation: f32, value: f32) -> [u8; 3] {
    let scaled = hue.rem_euclid(1.0) * 6.0;
    let sector = scaled.floor();
    let within = scaled - sector;
    let low = value * (1.0 - saturation);
    let falling = value * (1.0 - saturation * within);
    let rising = value * (1.0 - saturation * (1.0 - within));
    let rgb = match sector as u8 {
        0 => [value, rising, low],
        1 => [falling, value, low],
        2 => [low, value, rising],
        3 => [low, falling, value],
        4 => [rising, low, value],
        _ => [value, low, falling],
    };
    rgb.map(|c| (c * 255.0).round().clamp(0.0, 255.0) as u8)
}

/// An 8-bit RGB PNG of `rgb` (rows of `width` pixels).
fn png(width: u32, height: u32, rgb: &[u8]) -> Vec<u8> {
    let row = width as usize * 3;
    let mut raw = Vec::with_capacity(rgb.len() + height as usize);
    for line in rgb.chunks(row) {
        raw.push(0); // no filter
        raw.extend_from_slice(line);
    }
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    chunk(&mut out, *b"IHDR", &ihdr);
    chunk(&mut out, *b"IDAT", &zlib_stored(&raw));
    chunk(&mut out, *b"IEND", &[]);
    out
}

fn chunk(out: &mut Vec<u8>, kind: [u8; 4], data: &[u8]) {
    out.extend_from_slice(&u32::try_from(data.len()).unwrap_or(0).to_be_bytes());
    let mut body = kind.to_vec();
    body.extend_from_slice(data);
    out.extend_from_slice(&body);
    out.extend_from_slice(&crc32(&body).to_be_bytes());
}

/// A zlib stream of stored deflate blocks.
fn zlib_stored(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01];
    let blocks: Vec<&[u8]> = data.chunks(0xffff).collect();
    for (n, block) in blocks.iter().enumerate() {
        out.push(u8::from(n + 1 == blocks.len()));
        let len = u16::try_from(block.len()).unwrap_or(u16::MAX);
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&(!len).to_le_bytes());
        out.extend_from_slice(block);
    }
    let (a, b) = data.iter().fold((1_u32, 0_u32), |(a, b), &byte| {
        let a = (a + u32::from(byte)) % 65_521;
        (a, (b + a) % 65_521)
    });
    out.extend_from_slice(&((b << 16) | a).to_be_bytes());
    out
}

fn crc32(data: &[u8]) -> u32 {
    !data.iter().fold(!0_u32, |crc, &byte| {
        (0..8).fold(crc ^ u32::from(byte), |c, _| {
            if c & 1 == 1 {
                (c >> 1) ^ 0xedb8_8320
            } else {
                c >> 1
            }
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_framing_is_valid() {
        // Known CRC-32 and Adler-32 of "IEND" / "Wikipedia".
        assert_eq!(crc32(b"IEND"), 0xae42_6082);
        let stream = zlib_stored(b"Wikipedia");
        assert_eq!(&stream[stream.len() - 4..], &0x11e6_0398_u32.to_be_bytes());
        let image = png(2, 1, &[255, 0, 0, 0, 0, 255]);
        assert!(image.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert!(image.ends_with(&[0xae, 0x42, 0x60, 0x82]));
    }

    #[test]
    fn people_with_names_and_groups_have_pictures_numbers_dont() {
        let world = World::build();
        let named = world
            .contacts
            .iter()
            .find(|c| c.name.is_some() || c.profile_name.is_some())
            .unwrap();
        let one = picture(&world, ConversationId::Direct(named.service_id)).unwrap();
        assert_eq!(
            picture(&world, ConversationId::Direct(named.service_id)),
            Some(one),
            "the same picture every time"
        );
        let group = world.groups.first().unwrap();
        assert!(picture(&world, ConversationId::Group(group.id)).is_some());
        if let Some(number) = world
            .contacts
            .iter()
            .find(|c| c.name.is_none() && c.profile_name.is_none())
        {
            assert_eq!(
                picture(&world, ConversationId::Direct(number.service_id)),
                None
            );
        }
    }
}
