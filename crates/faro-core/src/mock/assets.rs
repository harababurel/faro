//! The demo's attachments: photos, a sticker, text files and voice notes.
//!
//! Photos and the sticker are original images generated for faro (objects and
//! places only), bundled from `assets/demo/`. Text files are written in their
//! sender's voice. Voice notes are synthesized tones (there is nothing to hear,
//! only a waveform to draw).

use crate::domain::{AttachmentId, AttachmentRef, AttachmentStatus};

/// A canned attachment a scene line can carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum Asset {
    /// A stapler set in a block of yellow jello (Jim's prank).
    Stapler,
    /// Pam's watercolour of the office park.
    OfficePark,
    /// One of Angela's cats.
    Cat,
    /// Dwight's beet field at dawn.
    BeetField,
    /// Kevin's chili.
    Chili,
    /// Pretzel day.
    Pretzel,
    /// Michael's gold trophy sticker.
    Trophy,
    /// Dwight's checklist for the temp.
    EmergencyProtocol,
    /// Michael's Dundies list.
    DundiesNominees,
    /// Tanaka-san's invoice.
    Invoice,
    /// Michael's voice notes.
    MichaelVoice,
    /// Andy's a cappella voice notes.
    AndyVoice,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    Photo,
    Sticker,
    File,
    Voice,
}

pub(super) struct Spec {
    pub(super) id: &'static str,
    pub(super) content_type: &'static str,
    pub(super) file_name: Option<&'static str>,
    pub(super) dims: Option<(u32, u32)>,
    pub(super) kind: Kind,
}

/// Every asset, for lookups by ID and for tests.
pub(super) const ALL: &[Asset] = &[
    Asset::Stapler,
    Asset::OfficePark,
    Asset::Cat,
    Asset::BeetField,
    Asset::Chili,
    Asset::Pretzel,
    Asset::Trophy,
    Asset::EmergencyProtocol,
    Asset::DundiesNominees,
    Asset::Invoice,
    Asset::MichaelVoice,
    Asset::AndyVoice,
];

const fn photo(id: &'static str, file_name: &'static str) -> Spec {
    Spec {
        id,
        content_type: "image/jpeg",
        file_name: Some(file_name),
        dims: Some((320, 240)),
        kind: Kind::Photo,
    }
}

const fn text_file(id: &'static str, file_name: &'static str) -> Spec {
    Spec {
        id,
        content_type: "text/plain",
        file_name: Some(file_name),
        dims: None,
        kind: Kind::File,
    }
}

const fn voice(id: &'static str, file_name: &'static str) -> Spec {
    Spec {
        id,
        content_type: "audio/wav",
        file_name: Some(file_name),
        dims: None,
        kind: Kind::Voice,
    }
}

impl Asset {
    pub(super) const fn spec(self) -> Spec {
        match self {
            Self::Stapler => photo("mock:stapler-jello", "stapler-jello.jpg"),
            Self::OfficePark => photo("mock:office-park", "office-park-watercolour.jpg"),
            Self::Cat => photo("mock:grey-cat", "duchess.jpg"),
            Self::BeetField => photo("mock:beet-field", "beet-field.jpg"),
            Self::Chili => photo("mock:chili-pot", "chili-pot.jpg"),
            Self::Pretzel => photo("mock:pretzel", "pretzel.jpg"),
            // Like Signal stickers, it has no file name.
            Self::Trophy => Spec {
                id: "mock:sticker-trophy",
                content_type: "image/png",
                file_name: None,
                dims: Some((160, 160)),
                kind: Kind::Sticker,
            },
            Self::EmergencyProtocol => {
                text_file("mock:emergency-protocol", "emergency-protocol.txt")
            }
            Self::DundiesNominees => text_file("mock:dundies-nominees", "dundies-nominees.txt"),
            Self::Invoice => text_file("mock:invoice-4471", "invoice-4471.txt"),
            Self::MichaelVoice => voice("mock:voice-michael", "voice-michael.wav"),
            Self::AndyVoice => voice("mock:voice-andy", "voice-andy.wav"),
        }
    }

    /// A remote [`AttachmentRef`], as a backend would report it.
    pub(super) fn attachment(self) -> Option<AttachmentRef> {
        let spec = self.spec();
        let bytes = bytes(self);
        let audio = (spec.kind == Kind::Voice)
            .then(|| crate::audio::inspect_wav(&bytes))
            .flatten();
        Some(AttachmentRef {
            id: AttachmentId(spec.id.to_owned()),
            content_type: spec.content_type.to_owned(),
            file_name: spec.file_name.map(str::to_owned),
            size: Some(u64::try_from(bytes.len()).ok()?),
            width: spec.dims.map(|(w, _)| w),
            height: spec.dims.map(|(_, h)| h),
            status: AttachmentStatus::Remote,
            voice_note: spec.kind == Kind::Voice,
            duration_secs: audio.as_ref().and_then(|a| a.duration_secs),
            waveform: audio.map(|a| a.waveform),
            transcript: None,
            transcript_source: None,
        })
    }
}

/// Bytes served by [`super::MockBackend::fetch_attachment`] for `id`.
pub(super) fn attachment_bytes(id: &str) -> Option<Vec<u8>> {
    ALL.iter()
        .find(|a| a.spec().id == id)
        .map(|&asset| bytes(asset))
}

fn bytes(asset: Asset) -> Vec<u8> {
    match asset {
        Asset::Stapler => include_bytes!("../../assets/demo/stapler-jello.jpg").to_vec(),
        Asset::OfficePark => {
            include_bytes!("../../assets/demo/office-park-watercolour.jpg").to_vec()
        }
        Asset::Cat => include_bytes!("../../assets/demo/grey-cat.jpg").to_vec(),
        Asset::BeetField => include_bytes!("../../assets/demo/beet-field.jpg").to_vec(),
        Asset::Chili => include_bytes!("../../assets/demo/chili-pot.jpg").to_vec(),
        Asset::Pretzel => include_bytes!("../../assets/demo/pretzel.jpg").to_vec(),
        Asset::Trophy => include_bytes!("../../assets/demo/trophy-sticker.png").to_vec(),
        Asset::EmergencyProtocol => EMERGENCY_PROTOCOL.as_bytes().to_vec(),
        Asset::DundiesNominees => DUNDIES_NOMINEES.as_bytes().to_vec(),
        Asset::Invoice => INVOICE.as_bytes().to_vec(),
        Asset::MichaelVoice => encode_voice_wav(3, &MICHAEL_ENVELOPE),
        Asset::AndyVoice => encode_voice_wav(5, &ANDY_ENVELOPE),
    }
}

const EMERGENCY_PROTOCOL: &str = "\
EMERGENCY PROTOCOL — SCRANTON BRANCH
Issued to: the temp
Issued by: Dwight K. Schrute III, Assistant (to the) Regional Manager

1. Memorize all exits. There are four. The fifth is classified.
2. In case of fire: stay low, follow Dwight, do not help Michael carry the TV.
3. In case of bear: do NOT run. Bears can climb, swim and smell fear.
4. In case of corporate visit: look busy. Look VERY busy.
5. Your desk drawer contains: one granola bar, one whistle. Do not eat the whistle.
6. Report all suspicious behaviour (Jim) to Dwight immediately.

Sign below to confirm you have read and understood this protocol.

Signature: ______________________
";

const DUNDIES_NOMINEES: &str = "\
THE DUNDIES — OFFICIAL NOMINEES (CONFIDENTIAL!!)

Best Dundie Award ............ everybody (it's a tie, you are all winners)
Spicy Curry Award ............ Kelly
Fine Work Award .............. Stanley (for his fine work)
Hottest in the Office ........ Ryan (again, sorry everyone else)
Don't Go In There After Me ... Kevin
Best New Temp ................ the temp!!! (only nominee, you already won)

Ceremony at the restaurant, 7pm. Dress code: formal-ish. Bring a speech.
";

const INVOICE: &str = "\
INVOICE No. 4471
From: Tanaka Paper Mill Co., Ltd. (Fuji, Shizuoka)
To:   Scranton branch — attn. the temp

  500 boxes  letter-size copy paper, 20 lb, white
   40 boxes  cardstock, 110 lb, ivory

Payment terms: net 30
Thank you for your continued business.
";

const MICHAEL_ENVELOPE: [i16; 16] = [
    1_800, 4_200, 7_500, 9_800, 6_100, 2_400, 8_200, 10_500, 5_400, 3_100, 7_000, 9_100, 6_400,
    4_500, 2_900, 1_400,
];

const ANDY_ENVELOPE: [i16; 16] = [
    3_000, 5_500, 8_000, 9_500, 10_500, 9_500, 8_000, 6_000, 7_000, 9_000, 11_000, 12_000, 10_000,
    7_500, 4_500, 2_000,
];

/// An 8 kHz mono 16-bit PCM WAV of `seconds` with a speech-like `envelope`.
fn encode_voice_wav(seconds: u32, envelope: &[i16]) -> Vec<u8> {
    const SAMPLE_RATE: u32 = 8_000;
    let total_samples = usize::try_from(SAMPLE_RATE * seconds).unwrap_or(0);
    let data_len = u32::try_from(total_samples * 2).unwrap_or(0);
    let riff_len = 36_u32.saturating_add(data_len);
    let byte_rate = SAMPLE_RATE * 2;
    let mut wav = Vec::with_capacity(44 + total_samples * 2);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&riff_len.to_le_bytes());
    wav.extend_from_slice(b"WAVE");
    wav.extend_from_slice(b"fmt ");
    wav.extend_from_slice(&16_u32.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    wav.extend_from_slice(&byte_rate.to_le_bytes());
    wav.extend_from_slice(&2_u16.to_le_bytes());
    wav.extend_from_slice(&16_u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    for i in 0..total_samples {
        let bucket = i * envelope.len() / total_samples.max(1);
        let gain = envelope.get(bucket).copied().unwrap_or(0);
        let sign = if (i / 18).is_multiple_of(2) {
            1_i16
        } else {
            -1_i16
        };
        let sample = sign.saturating_mul(gain);
        wav.extend_from_slice(&sample.to_le_bytes());
    }
    wav
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn canned_attachments_have_matching_bytes() {
        let ids: HashSet<&str> = ALL.iter().map(|a| a.spec().id).collect();
        assert_eq!(ids.len(), ALL.len(), "asset IDs are unique");
        for &asset in ALL {
            let att = asset.attachment().expect("every asset has an attachment");
            let bytes = attachment_bytes(&att.id.0).expect("bytes exist");
            assert_eq!(att.size, Some(bytes.len() as u64));
            match asset.spec().kind {
                Kind::Photo | Kind::Sticker => {
                    assert!(att.is_image(), "{asset:?} is an image");
                    let png = bytes.starts_with(b"\x89PNG\r\n\x1a\n");
                    let jpeg = bytes.starts_with(&[0xff, 0xd8, 0xff]);
                    let webp = bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP");
                    assert!(png || jpeg || webp, "{asset:?} has an image signature");
                    assert!(att.width.is_some() && att.height.is_some());
                }
                Kind::Voice => {
                    assert!(att.is_voice_note());
                    assert!(att.duration_secs.is_some() && att.waveform.is_some());
                }
                Kind::File => {
                    assert!(std::str::from_utf8(&bytes).is_ok(), "{asset:?} is text");
                    assert!(att.file_name.is_some());
                }
            }
        }
        assert_eq!(attachment_bytes("mock:nothing"), None);
    }
}
