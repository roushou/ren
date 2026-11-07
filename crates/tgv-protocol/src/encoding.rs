//! Body format and content encoding, and incremental decompression.
//!
//! Only classifications are kept from headers, never header values. Decoding
//! is incremental, so compressed bodies are never buffered whole, and it
//! distinguishes a corrupt stream from one that simply ended early.

use http::{HeaderMap, header};
use serde_with::{DeserializeFromStr, SerializeDisplay};
use std::io::Write;
use strum::{Display, EnumString, IntoStaticStr, VariantArray};

/// How the response body is framed: from `Content-Type` over HTTP, or
/// WebSocket messages, one JSON event each, recorded back to back.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum ResponseFormat {
    EventStream,
    Json,
    Other,
    /// No `Content-Type`.
    Unspecified,
    #[strum(to_string = "websocket_messages")]
    WebSocketMessages,
}

impl ResponseFormat {
    pub fn from_headers(headers: &HeaderMap) -> Self {
        let mut values = headers.get_all(header::CONTENT_TYPE).iter();
        let first = values.next();
        if values.next().is_some() {
            return Self::Other;
        }
        let Some(value) = first else {
            return Self::Unspecified;
        };
        let media_type = value
            .to_str()
            .ok()
            .and_then(|value| value.split(';').next())
            .map(str::trim);
        match media_type {
            Some(media) if media.eq_ignore_ascii_case("text/event-stream") => Self::EventStream,
            Some(media) if media.eq_ignore_ascii_case("application/json") => Self::Json,
            Some(_) | None => Self::Other,
        }
    }
}

/// How the response body is encoded, from `Content-Encoding`.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum ResponseEncoding {
    Identity,
    Gzip,
    Zstd,
    /// `Content-Encoding: br`.
    Brotli,
    /// Several encodings, or one TGV does not decode.
    Unsupported,
}

impl ResponseEncoding {
    /// A compressed body's encoding recognised by its magic bytes: gzip or
    /// zstd. Brotli has none, so it is never sniffed.
    pub fn sniff(bytes: &[u8]) -> Option<Self> {
        if bytes.starts_with(&[0x1f, 0x8b]) {
            Some(Self::Gzip)
        } else if bytes.starts_with(&[0x28, 0xb5, 0x2f, 0xfd]) {
            Some(Self::Zstd)
        } else {
            None
        }
    }

    pub fn from_headers(headers: &HeaderMap) -> Self {
        let mut values = headers.get_all(header::CONTENT_ENCODING).iter();
        let first = values.next();
        if values.next().is_some() {
            return Self::Unsupported;
        }
        let Some(value) = first else {
            return Self::Identity;
        };
        let value = value.as_bytes().trim_ascii();
        if value.eq_ignore_ascii_case(b"identity") {
            Self::Identity
        } else if value.eq_ignore_ascii_case(b"gzip") {
            Self::Gzip
        } else if value.eq_ignore_ascii_case(b"zstd") {
            Self::Zstd
        } else if value.eq_ignore_ascii_case(b"br") {
            Self::Brotli
        } else {
            Self::Unsupported
        }
    }
}

/// Why decoding stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum DecodeFailure {
    #[error("compressed stream is corrupt")]
    Corrupt,
    #[error("compressed stream ended before its end marker")]
    Truncated,
    #[error("content encoding is not supported")]
    Unsupported,
}

/// Incremental decoder for one body.
pub struct Decompressor {
    codec: Codec,
}

enum Codec {
    Identity,
    Gzip(Box<flate2::write::MultiGzDecoder<Vec<u8>>>),
    Zstd(Box<ZstdStream>),
    Brotli(Box<brotli_decompressor::DecompressorWriter<Vec<u8>>>),
}

/// Brotli's working buffer; the window it decodes with is the stream's own,
/// at most 16 MiB (large windows are not enabled).
const BROTLI_BUFFER: usize = 32 * 1024;

struct ZstdStream {
    decoder: zstd::stream::raw::Decoder<'static>,
    frame: FrameState,
}

/// Whether input so far ends exactly at a zstd frame boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FrameState {
    Open,
    Complete,
}

impl ZstdStream {
    /// Same window bound as the body preview: 8 MiB of history.
    const WINDOW_LOG_MAX: u32 = 23;
    const OUTPUT_CHUNK: usize = 32 * 1024;
}

impl std::fmt::Debug for Decompressor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let codec = match self.codec {
            Codec::Identity => "identity",
            Codec::Gzip(_) => "gzip",
            Codec::Zstd(_) => "zstd",
            Codec::Brotli(_) => "brotli",
        };
        f.debug_struct("Decompressor")
            .field("codec", &codec)
            .finish()
    }
}

impl Decompressor {
    pub fn new(encoding: ResponseEncoding) -> Result<Self, DecodeFailure> {
        let codec = match encoding {
            ResponseEncoding::Identity => Codec::Identity,
            ResponseEncoding::Gzip => {
                Codec::Gzip(Box::new(flate2::write::MultiGzDecoder::new(Vec::new())))
            }
            ResponseEncoding::Zstd => {
                let mut decoder =
                    zstd::stream::raw::Decoder::new().map_err(|_| DecodeFailure::Corrupt)?;
                decoder
                    .set_parameter(zstd::stream::raw::DParameter::WindowLogMax(
                        ZstdStream::WINDOW_LOG_MAX,
                    ))
                    .map_err(|_| DecodeFailure::Corrupt)?;
                Codec::Zstd(Box::new(ZstdStream {
                    decoder,
                    frame: FrameState::Open,
                }))
            }
            ResponseEncoding::Brotli => Codec::Brotli(Box::new(
                brotli_decompressor::DecompressorWriter::new(Vec::new(), BROTLI_BUFFER),
            )),
            ResponseEncoding::Unsupported => return Err(DecodeFailure::Unsupported),
        };
        Ok(Self { codec })
    }

    /// Decode the next fragment, handing decoded bytes to `out` in order.
    pub fn decode(
        &mut self,
        input: &[u8],
        mut out: impl FnMut(&[u8]),
    ) -> Result<(), DecodeFailure> {
        match &mut self.codec {
            Codec::Identity => {
                out(input);
                Ok(())
            }
            Codec::Gzip(decoder) => {
                decoder
                    .write_all(input)
                    .map_err(|_| DecodeFailure::Corrupt)?;
                let decoded = decoder.get_mut();
                out(decoded);
                decoded.clear();
                Ok(())
            }
            Codec::Zstd(stream) => stream.run(input, &mut out),
            Codec::Brotli(decoder) => {
                decoder
                    .write_all(input)
                    .map_err(|_| DecodeFailure::Corrupt)?;
                let decoded = decoder.get_mut();
                out(decoded);
                decoded.clear();
                Ok(())
            }
        }
    }

    /// End of input. Reports whether the stream ended at a frame boundary.
    pub fn finish(self, mut out: impl FnMut(&[u8])) -> Result<(), DecodeFailure> {
        match self.codec {
            Codec::Identity => Ok(()),
            Codec::Gzip(mut decoder) => {
                let result = decoder.try_finish();
                let decoded = decoder.get_mut();
                out(decoded);
                decoded.clear();
                result.map_err(|_| DecodeFailure::Truncated)
            }
            Codec::Zstd(stream) => match stream.frame {
                FrameState::Complete => Ok(()),
                FrameState::Open => Err(DecodeFailure::Truncated),
            },
            Codec::Brotli(mut decoder) => {
                // `close` fails unless the stream reached its final block.
                let result = decoder.close();
                let decoded = decoder.get_mut();
                out(decoded);
                decoded.clear();
                result.map_err(|_| DecodeFailure::Truncated)
            }
        }
    }
}

impl ZstdStream {
    fn run(&mut self, input: &[u8], out: &mut impl FnMut(&[u8])) -> Result<(), DecodeFailure> {
        use zstd::stream::raw::{InBuffer, Operation, OutBuffer};
        let mut input = InBuffer::around(input);
        let mut buffer = vec![0; Self::OUTPUT_CHUNK];
        loop {
            let mut output = OutBuffer::around(buffer.as_mut_slice());
            let consumed_before = input.pos();
            let hint = self
                .decoder
                .run(&mut input, &mut output)
                .map_err(|_| DecodeFailure::Corrupt)?;
            // A zero hint means a frame just ended. Calls that consume no input
            // (an empty fragment, or draining output) can report a nonzero hint
            // for the next frame without opening it.
            if hint == 0 {
                self.frame = FrameState::Complete;
            } else if input.pos() > consumed_before {
                self.frame = FrameState::Open;
            }
            let written = output.pos();
            out(&buffer[..written]);
            let input_done = input.pos() == input.src.len();
            if input_done && written < buffer.len() {
                return Ok(());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{DecodeFailure, Decompressor, ResponseEncoding, ResponseFormat};
    use http::{HeaderMap, HeaderValue, header};
    use std::io::Write;

    fn brotli(bytes: &[u8]) -> Vec<u8> {
        let mut encoded = Vec::new();
        {
            let mut encoder = brotli::CompressorWriter::new(&mut encoded, 4096, 5, 22);
            encoder.write_all(bytes).unwrap();
        }
        encoded
    }

    fn gzip(bytes: &[u8]) -> Vec<u8> {
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(bytes).unwrap();
        encoder.finish().unwrap()
    }

    fn decode(
        encoding: ResponseEncoding,
        input: &[u8],
        chunk: usize,
    ) -> (Vec<u8>, Result<(), DecodeFailure>) {
        let mut decoder = Decompressor::new(encoding).unwrap();
        let mut decoded = Vec::new();
        for part in input.chunks(chunk.max(1)) {
            if let Err(error) = decoder.decode(part, |bytes| decoded.extend_from_slice(bytes)) {
                return (decoded, Err(error));
            }
        }
        let end = decoder.finish(|bytes| decoded.extend_from_slice(bytes));
        (decoded, end)
    }

    #[test]
    fn magic_bytes_name_gzip_and_zstd_only() {
        assert_eq!(
            ResponseEncoding::sniff(&[0x1f, 0x8b, 8]),
            Some(ResponseEncoding::Gzip)
        );
        assert_eq!(
            ResponseEncoding::sniff(&[0x28, 0xb5, 0x2f, 0xfd, 0]),
            Some(ResponseEncoding::Zstd)
        );
        assert_eq!(ResponseEncoding::sniff(b"{\"model\":1}"), None);
        assert_eq!(ResponseEncoding::sniff(&[0x1f]), None);
    }

    #[test]
    fn classifies_formats_and_encodings_from_single_headers() {
        let headers = |name: header::HeaderName, values: &[&'static str]| {
            let mut map = HeaderMap::new();
            for value in values {
                map.append(&name, HeaderValue::from_static(value));
            }
            map
        };
        assert_eq!(
            ResponseFormat::from_headers(&headers(
                header::CONTENT_TYPE,
                &["Text/Event-Stream; charset=utf-8"]
            )),
            ResponseFormat::EventStream
        );
        assert_eq!(
            ResponseFormat::from_headers(&headers(
                header::CONTENT_TYPE,
                &["application/json", "text/plain"]
            )),
            ResponseFormat::Other
        );
        assert_eq!(
            ResponseFormat::from_headers(&HeaderMap::new()),
            ResponseFormat::Unspecified
        );
        assert_eq!(
            ResponseEncoding::from_headers(&headers(header::CONTENT_ENCODING, &["ZSTD"])),
            ResponseEncoding::Zstd
        );
        assert_eq!(
            ResponseEncoding::from_headers(&headers(header::CONTENT_ENCODING, &["gzip, br"])),
            ResponseEncoding::Unsupported
        );
        assert_eq!(
            ResponseEncoding::from_headers(&HeaderMap::new()),
            ResponseEncoding::Identity
        );
    }

    #[test]
    fn gzip_and_zstd_decode_incrementally_and_detect_truncation() {
        let body = b"data: {\"type\":\"message_stop\"}\n\n".repeat(64);
        let zstd = zstd::stream::encode_all(&body[..], 3).unwrap();
        for (encoding, encoded) in [
            (ResponseEncoding::Gzip, gzip(&body)),
            (ResponseEncoding::Zstd, zstd),
        ] {
            for chunk in [1, 7, encoded.len()] {
                assert_eq!(decode(encoding, &encoded, chunk), (body.clone(), Ok(())));
            }
            let (prefix, end) = decode(encoding, &encoded[..encoded.len() / 2], 5);
            assert_eq!(end, Err(DecodeFailure::Truncated));
            assert!(body.starts_with(&prefix));
            let mut corrupt = encoded.clone();
            corrupt[0] ^= 0xff;
            assert_eq!(
                decode(encoding, &corrupt, 64).1,
                Err(DecodeFailure::Corrupt)
            );
        }
        assert_eq!(
            decode(ResponseEncoding::Identity, b"plain", 2),
            (b"plain".to_vec(), Ok(()))
        );
    }

    #[test]
    fn empty_fragments_after_a_complete_frame_do_not_reopen_it() {
        for encoding in [
            ResponseEncoding::Gzip,
            ResponseEncoding::Zstd,
            ResponseEncoding::Brotli,
        ] {
            let encoded = match encoding {
                ResponseEncoding::Gzip => gzip(b"payload"),
                ResponseEncoding::Brotli => brotli(b"payload"),
                ResponseEncoding::Zstd
                | ResponseEncoding::Identity
                | ResponseEncoding::Unsupported => {
                    zstd::stream::encode_all(&b"payload"[..], 3).unwrap()
                }
            };
            let mut decoder = Decompressor::new(encoding).unwrap();
            let mut decoded = Vec::new();
            for part in [&encoded[..], &[][..], &[][..]] {
                decoder
                    .decode(part, |bytes| decoded.extend_from_slice(bytes))
                    .unwrap();
            }
            assert_eq!(
                decoder.finish(|bytes| decoded.extend_from_slice(bytes)),
                Ok(())
            );
            assert_eq!(decoded, b"payload");
        }
        assert_eq!(
            Decompressor::new(ResponseEncoding::Unsupported).unwrap_err(),
            DecodeFailure::Unsupported
        );
    }

    #[test]
    fn brotli_decodes_incrementally_and_detects_truncation_and_corruption() {
        let body: Vec<u8> =
            br#"{"type":"error","error":{"type":"rate_limit_error","message":"Error"}}"#
                .iter()
                .copied()
                .cycle()
                .take(40_000)
                .collect();
        let encoded = brotli(&body);
        for chunk in [1, 7, 4096, encoded.len()] {
            let (decoded, end) = decode(ResponseEncoding::Brotli, &encoded, chunk);
            assert_eq!(end, Ok(()), "chunk {chunk}");
            assert_eq!(decoded, body, "chunk {chunk}");
        }
        let cut = &encoded[..encoded.len() - 3];
        let (decoded, end) = decode(ResponseEncoding::Brotli, cut, 64);
        assert_eq!(end, Err(DecodeFailure::Truncated));
        assert!(body.starts_with(&decoded));
        let mut corrupt = encoded.clone();
        corrupt[0] = 0xff;
        corrupt[1] = 0xff;
        let (_, end) = decode(ResponseEncoding::Brotli, &corrupt, 64);
        assert!(end.is_err());
        let mut headers = HeaderMap::new();
        headers.insert(header::CONTENT_ENCODING, HeaderValue::from_static("BR"));
        assert_eq!(
            ResponseEncoding::from_headers(&headers),
            ResponseEncoding::Brotli
        );
    }
}
