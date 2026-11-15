//! Body format and content encoding, and incremental decompression.
//!
//! Only classifications are kept from headers, never header values. Decoding
//! is incremental, so compressed bodies are never buffered whole, and it
//! distinguishes a corrupt stream from one that simply ended early. Output
//! is handed on in bounded pieces: however much a fragment expands, no
//! buffer grows with it.

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
    /// Several encodings, or one Ren does not decode.
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
    Brotli(Box<BrotliStream>),
}

/// Gzip input is written this many bytes at a time. Deflate expands a byte
/// at most 1032 times, so the decoder's output buffer stays near 260 KiB
/// whatever the stream holds.
const GZIP_INPUT_STEP: usize = 256;

/// Brotli decoded through its streaming interface, into a fixed output
/// buffer. Its window is the stream's own, at most 16 MiB: large windows
/// are refused (`BrotliState::new_strict`).
struct BrotliStream {
    state: brotli_decompressor::BrotliState<
        brotli_decompressor::StandardAlloc,
        brotli_decompressor::StandardAlloc,
        brotli_decompressor::StandardAlloc,
    >,
    frame: FrameState,
}

impl BrotliStream {
    const OUTPUT_CHUNK: usize = 32 * 1024;
}

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
            ResponseEncoding::Brotli => Codec::Brotli(Box::new(BrotliStream::new())),
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
                for step in input.chunks(GZIP_INPUT_STEP) {
                    decoder
                        .write_all(step)
                        .map_err(|_| DecodeFailure::Corrupt)?;
                    let decoded = decoder.get_mut();
                    out(decoded);
                    decoded.clear();
                }
                Ok(())
            }
            Codec::Zstd(stream) => stream.run(input, &mut out),
            Codec::Brotli(stream) => stream.run(input, &mut out),
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
            Codec::Brotli(stream) => match stream.frame {
                FrameState::Complete => Ok(()),
                FrameState::Open => Err(DecodeFailure::Truncated),
            },
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

/// A whole body read through its decoder, as `std::io::Read`: what the
/// reader asks for is decoded, and nothing more, so a body is never held
/// decoded however far it expands. Reading a slice is not I/O.
pub struct BodyReader<'a> {
    source: Source<'a>,
}

enum Source<'a> {
    Identity(&'a [u8]),
    Gzip(Box<flate2::bufread::MultiGzDecoder<&'a [u8]>>),
    Zstd(Box<zstd::stream::read::Decoder<'static, &'a [u8]>>),
    Brotli(Box<BrotliPull<'a>>),
}

/// Brotli pulled straight into the reader's buffer.
struct BrotliPull<'a> {
    input: &'a [u8],
    stream: BrotliStream,
}

impl<'a> BodyReader<'a> {
    pub fn new(encoding: ResponseEncoding, body: &'a [u8]) -> Result<Self, DecodeFailure> {
        let source = match encoding {
            ResponseEncoding::Identity => Source::Identity(body),
            ResponseEncoding::Gzip => {
                Source::Gzip(Box::new(flate2::bufread::MultiGzDecoder::new(body)))
            }
            ResponseEncoding::Zstd => {
                let mut decoder = zstd::stream::read::Decoder::with_buffer(body)
                    .map_err(|_| DecodeFailure::Corrupt)?;
                decoder
                    .window_log_max(ZstdStream::WINDOW_LOG_MAX)
                    .map_err(|_| DecodeFailure::Corrupt)?;
                Source::Zstd(Box::new(decoder))
            }
            ResponseEncoding::Brotli => Source::Brotli(Box::new(BrotliPull {
                input: body,
                stream: BrotliStream::new(),
            })),
            ResponseEncoding::Unsupported => return Err(DecodeFailure::Unsupported),
        };
        Ok(Self { source })
    }
}

impl std::io::Read for BodyReader<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        match &mut self.source {
            Source::Identity(rest) => rest.read(buffer),
            Source::Gzip(decoder) => decoder.read(buffer),
            Source::Zstd(decoder) => decoder.read(buffer),
            Source::Brotli(pull) => pull.read(buffer),
        }
    }
}

impl std::io::Read for BrotliPull<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let corrupt =
            |failure: DecodeFailure| std::io::Error::new(std::io::ErrorKind::InvalidData, failure);
        if buffer.is_empty() {
            return Ok(0);
        }
        loop {
            let (consumed, produced) = self.stream.step(self.input, buffer).map_err(corrupt)?;
            self.input = self.input.get(consumed..).unwrap_or_default();
            match (produced, self.stream.frame) {
                // Input taken, nothing out yet: keep going.
                (0, FrameState::Open) if consumed > 0 => {}
                (0, FrameState::Open) => return Err(corrupt(DecodeFailure::Truncated)),
                (0, FrameState::Complete) if !self.input.is_empty() => {
                    return Err(corrupt(DecodeFailure::Corrupt));
                }
                (produced, FrameState::Open | FrameState::Complete) => return Ok(produced),
            }
        }
    }
}

impl BrotliStream {
    fn new() -> Self {
        Self {
            state: brotli_decompressor::BrotliState::new_strict(
                brotli_decompressor::StandardAlloc::default(),
                brotli_decompressor::StandardAlloc::default(),
                brotli_decompressor::StandardAlloc::default(),
            ),
            frame: FrameState::Open,
        }
    }

    /// Decode from `input` into `output`, at most its length: the input
    /// consumed and the bytes produced.
    fn step(&mut self, input: &[u8], output: &mut [u8]) -> Result<(usize, usize), DecodeFailure> {
        use brotli_decompressor::{BrotliDecompressStream, BrotliResult};
        if self.frame == FrameState::Complete {
            return Ok((0, 0));
        }
        let mut available_in = input.len();
        let mut input_offset = 0;
        let mut available_out = output.len();
        let mut output_offset = 0;
        let mut total_out = 0;
        let result = BrotliDecompressStream(
            &mut available_in,
            &mut input_offset,
            input,
            &mut available_out,
            &mut output_offset,
            output,
            &mut total_out,
            &mut self.state,
        );
        match result {
            BrotliResult::NeedsMoreOutput | BrotliResult::NeedsMoreInput => {}
            BrotliResult::ResultSuccess => self.frame = FrameState::Complete,
            BrotliResult::ResultFailure => return Err(DecodeFailure::Corrupt),
        }
        Ok((input_offset, output_offset))
    }

    fn run(&mut self, mut input: &[u8], out: &mut impl FnMut(&[u8])) -> Result<(), DecodeFailure> {
        if input.is_empty() {
            return Ok(());
        }
        // A stream ends once; bytes after its end are not brotli.
        if self.frame == FrameState::Complete {
            return Err(DecodeFailure::Corrupt);
        }
        let mut buffer = vec![0; Self::OUTPUT_CHUNK];
        loop {
            let (consumed, produced) = self.step(input, &mut buffer)?;
            input = input.get(consumed..).unwrap_or_default();
            out(buffer.get(..produced).unwrap_or_default());
            match (self.frame, input.is_empty()) {
                // More output is waiting while the buffer comes back full,
                // and input is left while the decoder still takes it.
                (FrameState::Open, _) if produced == buffer.len() => {}
                (FrameState::Open, false) if consumed > 0 => {}
                (FrameState::Open, _) => return Ok(()),
                (FrameState::Complete, true) => return Ok(()),
                (FrameState::Complete, false) => return Err(DecodeFailure::Corrupt),
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

    #[test]
    fn a_small_fragment_that_expands_hugely_arrives_in_bounded_pieces() {
        let zstd = |bytes: &[u8]| zstd::stream::encode_all(bytes, 19).unwrap();
        let body = vec![0_u8; 16 * 1024 * 1024];
        for (encoding, encoded) in [
            (ResponseEncoding::Gzip, gzip(&body)),
            (ResponseEncoding::Zstd, zstd(&body)),
            (ResponseEncoding::Brotli, brotli(&body)),
        ] {
            // The whole bomb in one fragment.
            let mut decoder = Decompressor::new(encoding).unwrap();
            let mut largest = 0;
            let mut total = 0;
            let mut piece = |bytes: &[u8]| {
                largest = largest.max(bytes.len());
                total += bytes.len();
            };
            decoder.decode(&encoded, &mut piece).unwrap();
            decoder.finish(&mut piece).unwrap();
            assert_eq!(total, body.len(), "{encoding}");
            assert!(largest <= 300 * 1024, "{encoding}: a {largest}-byte piece");
        }
    }

    #[test]
    fn bytes_after_a_brotli_stream_ends_are_corrupt() {
        let mut encoded = brotli(b"{}");
        encoded.extend_from_slice(b"tail");
        let (decoded, end) = decode(ResponseEncoding::Brotli, &encoded, encoded.len());
        assert_eq!(decoded, b"{}");
        assert_eq!(end, Err(DecodeFailure::Corrupt));
    }

    #[test]
    fn a_body_reader_reads_each_encoding_and_never_holds_more_than_asked() {
        use super::BodyReader;
        use std::io::Read;
        let body: Vec<u8> =
            br#"{"model":"m-1","input":"ok"}"#.iter().copied().cycle().take(200_000).collect();
        let zstd = zstd::stream::encode_all(&body[..], 3).unwrap();
        for (encoding, encoded) in [
            (ResponseEncoding::Identity, body.clone()),
            (ResponseEncoding::Gzip, gzip(&body)),
            (ResponseEncoding::Zstd, zstd),
            (ResponseEncoding::Brotli, brotli(&body)),
        ] {
            let mut read = Vec::new();
            BodyReader::new(encoding, &encoded)
                .unwrap()
                .read_to_end(&mut read)
                .unwrap();
            assert_eq!(read, body, "{encoding}");
            // A bounded read stops at its bound.
            let mut head = Vec::new();
            BodyReader::new(encoding, &encoded)
                .unwrap()
                .take(10)
                .read_to_end(&mut head)
                .unwrap();
            assert_eq!(head, body[..10], "{encoding}");
            // Cut short, it fails rather than ending quietly.
            let cut = &encoded[..encoded.len() / 2];
            let mut rest = Vec::new();
            let ended = BodyReader::new(encoding, cut)
                .unwrap()
                .read_to_end(&mut rest);
            assert_eq!(
                ended.is_err(),
                encoding != ResponseEncoding::Identity,
                "{encoding}"
            );
        }
        assert!(BodyReader::new(ResponseEncoding::Unsupported, b"").is_err());
    }
}
