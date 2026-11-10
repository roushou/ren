//! Bounded presentation of a stored body prefix. Never used by forwarding.
//!
//! Gzip and zstd are recognised by their magic bytes. Brotli has none, so it
//! is decoded only when the body's attempt recorded `Content-Encoding: br`;
//! inspection always uses the recorded encoding. Decoded output is bounded, so a
//! small compressed prefix cannot expand without limit.

use ren_protocol::{encoding::ResponseEncoding, label::Stored};
use ren_recorder::capture::BodyEnding;
use ren_store::{PrefixLimit, view::BodyPrefix};
use serde::Serialize;
use serde_with::{DeserializeFromStr, SerializeDisplay};
use std::io::{self, Read};
use strum::{Display, EnumString, IntoStaticStr, VariantArray};

/// How the raw bytes are shown.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum RawFormat {
    Utf8,
    Hex,
}

/// Which decompression the preview applied.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum PreviewDecoding {
    #[strum(to_string = "none")]
    Plain,
    GzipMagic,
    ZstdMagic,
    /// From the recorded `Content-Encoding`, not from the bytes.
    Brotli,
}

/// How much of the body a preview shows.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum PreviewExtent {
    /// The whole body: every observed byte is stored and its end was seen.
    Complete,
    /// Every stored byte, but the body is still open or was not fully stored.
    Partial,
    /// The body continues beyond the preview limit.
    Truncated,
    /// The preview stops where stored bytes are missing.
    Gap,
}

/// Whether text ends cleanly.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum TextEnd {
    Intact,
    /// The text ends inside a multi-byte character.
    SplitCharacter,
}

/// What decompression established about the bytes.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum DecodeIntegrity {
    NotDecoded,
    Verified,
    /// The bytes decoded so far are valid, but the stream ended early.
    Prefix,
    Failed,
}

#[derive(Debug, Serialize)]
pub struct BodyPreview {
    /// Unknown while the body is still being received.
    pub observed_bytes: Option<u64>,
    pub stored_bytes: u64,
    pub ending: Stored<BodyEnding>,
    /// Bytes in this preview.
    pub captured_bytes: usize,
    pub extent: PreviewExtent,
    pub raw: RawPreview,
    pub text: Option<String>,
    pub text_end: TextEnd,
    pub decoding: DecodingPreview,
}

#[derive(Debug, Serialize)]
pub struct RawPreview {
    pub text: String,
    pub format: RawFormat,
    /// Hex output stops at the raw display limit.
    pub truncated: bool,
}

#[derive(Debug, Serialize)]
pub struct DecodingPreview {
    pub applied: PreviewDecoding,
    pub integrity: DecodeIntegrity,
    /// Decoded output stops at the text limit.
    pub truncated: bool,
}

/// Owns preview budgets.
#[derive(Debug, Clone, Copy)]
pub struct Previewer {
    read: PrefixLimit,
    text_limit: usize,
    raw_limit: usize,
}

impl Default for Previewer {
    fn default() -> Self {
        Self {
            read: PrefixLimit {
                bytes: 256 * 1024,
                segments: 256,
            },
            text_limit: 512 * 1024,
            raw_limit: 16 * 1024,
        }
    }
}

impl Previewer {
    pub fn read_limit(&self) -> PrefixLimit {
        self.read
    }

    pub fn preview(&self, prefix: BodyPrefix) -> BodyPreview {
        let bytes = prefix.bytes.as_slice();
        let extent = if prefix.gap {
            PreviewExtent::Gap
        } else if prefix.truncated {
            PreviewExtent::Truncated
        } else if prefix.body.complete() {
            PreviewExtent::Complete
        } else {
            PreviewExtent::Partial
        };
        let raw = RawContent::from_bytes(bytes, self.raw_limit);
        let (text, decoding) = match Compression::infer(bytes, prefix.encoding.as_ref()) {
            None => (
                Text::from_bytes(bytes),
                DecodingPreview {
                    applied: PreviewDecoding::Plain,
                    integrity: DecodeIntegrity::NotDecoded,
                    truncated: false,
                },
            ),
            Some(compression) => {
                let decoded = compression.decode(bytes, self.text_limit);
                let text = match decoded.integrity {
                    DecodeIntegrity::Failed | DecodeIntegrity::NotDecoded => Text::NONE,
                    DecodeIntegrity::Verified | DecodeIntegrity::Prefix => {
                        Text::from_bytes(&decoded.bytes)
                    }
                };
                (
                    text,
                    DecodingPreview {
                        applied: compression.label(),
                        integrity: decoded.integrity,
                        truncated: decoded.truncated,
                    },
                )
            }
        };
        BodyPreview {
            observed_bytes: prefix.body.observed_bytes,
            stored_bytes: prefix.body.stored_bytes,
            ending: prefix.body.ending.clone(),
            captured_bytes: bytes.len(),
            extent,
            raw: RawPreview {
                text: raw.text,
                format: raw.format,
                truncated: raw.truncated,
            },
            text: text.text,
            text_end: text.end,
            decoding,
        }
    }
}

struct RawContent {
    text: String,
    format: RawFormat,
    truncated: bool,
}

impl RawContent {
    fn from_bytes(bytes: &[u8], limit: usize) -> Self {
        if let Ok(text) = std::str::from_utf8(bytes)
            && !text.contains('\0')
        {
            return Self {
                text: text.to_owned(),
                format: RawFormat::Utf8,
                truncated: false,
            };
        }
        let shown = bytes.get(..limit.min(bytes.len())).unwrap_or_default();
        let text = shown
            .chunks(16)
            .map(|row| {
                row.iter()
                    .map(|byte| format!("{byte:02x} "))
                    .collect::<String>()
                    + "\n"
            })
            .collect();
        Self {
            text,
            format: RawFormat::Hex,
            truncated: bytes.len() > shown.len(),
        }
    }
}

struct Text {
    text: Option<String>,
    end: TextEnd,
}

impl Text {
    const NONE: Self = Self {
        text: None,
        end: TextEnd::Intact,
    };

    fn from_bytes(bytes: &[u8]) -> Self {
        let (text, end) = match std::str::from_utf8(bytes) {
            Ok(text) => (Some(text), TextEnd::Intact),
            Err(error) if error.error_len().is_none() => (
                bytes
                    .get(..error.valid_up_to())
                    .and_then(|valid| std::str::from_utf8(valid).ok()),
                TextEnd::SplitCharacter,
            ),
            Err(_) => (None, TextEnd::Intact),
        };
        Self {
            text: text.filter(|text| !text.contains('\0')).map(str::to_owned),
            end,
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum Compression {
    Gzip,
    Zstd,
    Brotli,
}

struct Decoded {
    bytes: Vec<u8>,
    truncated: bool,
    integrity: DecodeIntegrity,
}

impl Compression {
    /// Magic bytes first; then the recorded encoding, for Brotli only.
    fn infer(bytes: &[u8], recorded: Option<&Stored<ResponseEncoding>>) -> Option<Self> {
        match ResponseEncoding::sniff(bytes) {
            Some(ResponseEncoding::Gzip) => Some(Self::Gzip),
            Some(ResponseEncoding::Zstd) => Some(Self::Zstd),
            Some(
                ResponseEncoding::Identity
                | ResponseEncoding::Brotli
                | ResponseEncoding::Unsupported,
            )
            | None => match recorded {
                Some(Stored::Known(ResponseEncoding::Brotli)) => Some(Self::Brotli),
                Some(Stored::Known(
                    ResponseEncoding::Identity
                    | ResponseEncoding::Gzip
                    | ResponseEncoding::Zstd
                    | ResponseEncoding::Unsupported,
                ))
                | Some(Stored::Unknown(_))
                | None => None,
            },
        }
    }

    fn label(self) -> PreviewDecoding {
        match self {
            Self::Gzip => PreviewDecoding::GzipMagic,
            Self::Zstd => PreviewDecoding::ZstdMagic,
            Self::Brotli => PreviewDecoding::Brotli,
        }
    }

    fn reader(self, bytes: &[u8]) -> io::Result<Box<dyn Read + '_>> {
        match self {
            Self::Gzip => Ok(Box::new(flate2::read::MultiGzDecoder::new(bytes))),
            Self::Zstd => {
                let mut decoder = zstd::stream::read::Decoder::with_buffer(bytes)?;
                decoder.window_log_max(23)?;
                Ok(Box::new(decoder))
            }
            Self::Brotli => Ok(Box::new(brotli_decompressor::Decompressor::new(
                bytes,
                32 * 1024,
            ))),
        }
    }

    /// Decode at most `limit` bytes. Reading stops at the limit, so expansion
    /// is bounded by the limit, not by the compressed input.
    fn decode(self, bytes: &[u8], limit: usize) -> Decoded {
        let mut output = Vec::new();
        let result = self.reader(bytes).and_then(|reader| {
            reader
                .take(u64::try_from(limit).unwrap_or(u64::MAX).saturating_add(1))
                .read_to_end(&mut output)
        });
        let truncated = output.len() > limit;
        output.truncate(limit);
        let integrity = match result {
            Ok(_) => DecodeIntegrity::Verified,
            Err(_) if !output.is_empty() => DecodeIntegrity::Prefix,
            Err(_) => DecodeIntegrity::Failed,
        };
        Decoded {
            bytes: output,
            truncated,
            integrity,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{DecodeIntegrity, PreviewDecoding, PreviewExtent, Previewer, RawFormat, TextEnd};
    use ren_protocol::{encoding::ResponseEncoding, label::Stored};
    use ren_recorder::capture::BodyEnding;
    use ren_store::view::{BodyPrefix, BodyView};
    use std::io::Write;

    fn prefix(bytes: &[u8]) -> BodyPrefix {
        let len = u64::try_from(bytes.len()).unwrap();
        BodyPrefix {
            body: BodyView {
                observed_bytes: Some(len),
                stored_bytes: len,
                ending: Stored::Known(BodyEnding::Ended),
            },
            bytes: bytes.to_vec(),
            gap: false,
            truncated: false,
            encoding: None,
        }
    }

    #[test]
    fn brotli_is_decoded_only_when_its_attempt_recorded_it() {
        let text = br#"{"type":"error","error":{"type":"rate_limit_error"}}"#;
        let mut encoded = Vec::new();
        {
            let mut encoder = brotli::CompressorWriter::new(&mut encoded, 4096, 5, 22);
            encoder.write_all(text).unwrap();
        }
        let previewer = Previewer::default();
        let recorded = BodyPrefix {
            encoding: Some(Stored::Known(ResponseEncoding::Brotli)),
            ..prefix(&encoded)
        };
        let preview = previewer.preview(recorded.clone());
        assert_eq!(preview.text.as_deref().map(str::as_bytes), Some(&text[..]));
        assert_eq!(preview.decoding.applied, PreviewDecoding::Brotli);
        assert_eq!(preview.decoding.integrity, DecodeIntegrity::Verified);
        // The bytes alone do not say Brotli: without the record, no guessing.
        let unrecorded = previewer.preview(prefix(&encoded));
        assert_eq!(unrecorded.decoding.applied, PreviewDecoding::Plain);
        assert!(unrecorded.text.is_none());
        // A cut stream decodes only as an unverified prefix, or fails.
        let cut = previewer.preview(BodyPrefix {
            bytes: encoded[..encoded.len() - 2].to_vec(),
            ..recorded
        });
        assert_ne!(cut.decoding.integrity, DecodeIntegrity::Verified);
    }

    #[test]
    fn compressed_previews_are_bounded_and_keep_the_original_bytes() {
        let previewer = Previewer::default();
        let text = b"{\"input\":\"hello\"}";
        let zstd = zstd::stream::encode_all(&text[..], 1).unwrap();
        let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        gzip.write_all(text).unwrap();
        let gzip = gzip.finish().unwrap();
        for (data, decoding) in [
            (zstd, PreviewDecoding::ZstdMagic),
            (gzip, PreviewDecoding::GzipMagic),
        ] {
            let preview = previewer.preview(prefix(&data));
            assert_eq!(preview.text.as_deref(), Some("{\"input\":\"hello\"}"));
            assert_eq!(
                (preview.raw.format, preview.decoding.applied),
                (RawFormat::Hex, decoding)
            );
            assert_eq!(preview.decoding.integrity, DecodeIntegrity::Verified);
            assert_eq!(preview.extent, PreviewExtent::Complete);
            let partial = previewer.preview(prefix(&data[..data.len() - 2]));
            assert_ne!(partial.decoding.integrity, DecodeIntegrity::Verified);
        }
        let big = zstd::stream::encode_all(&vec![b'a'; 600 * 1024][..], 1).unwrap();
        let preview = previewer.preview(prefix(&big));
        assert!(preview.decoding.truncated);
        assert_eq!(preview.text.unwrap().len(), 512 * 1024);
        let split = previewer.preview(prefix(b"hello \xf0\x9f"));
        assert_eq!(split.text.as_deref(), Some("hello "));
        assert_eq!(split.text_end, TextEnd::SplitCharacter);
        assert_eq!(
            previewer
                .preview(prefix("hello 🚄".as_bytes()))
                .text
                .as_deref(),
            Some("hello 🚄")
        );
        assert!(previewer.preview(prefix(&[0xff, 0x00])).text.is_none());
    }
}
