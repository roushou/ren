//! The stored chunk format: a chunk's identity and how its bytes are kept in
//! `blobs.sqlite3`. The recorder cuts bodies into chunks
//! (`ren_recorder::chunk`); the store hashes, encodes, and decodes them.

use serde_with::{DeserializeFromStr, SerializeDisplay};
use strum::{Display, EnumString, IntoStaticStr, VariantArray};

/// The blake3 hash of a chunk's raw bytes: its identity in `blobs.sqlite3`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ChunkHash([u8; 32]);

impl ChunkHash {
    pub fn of(bytes: &[u8]) -> Self {
        Self(*blake3::hash(bytes).as_bytes())
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// A hash read back from storage.
    pub fn restore(bytes: &[u8]) -> Option<Self> {
        <[u8; 32]>::try_from(bytes).ok().map(Self)
    }
}

/// How a chunk's bytes are stored.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum Codec {
    /// Stored as-is, because compression did not make it smaller.
    Raw,
    Zstd,
}

/// A chunk ready to insert: hash, raw length, codec, and stored bytes.
#[derive(Debug)]
pub struct EncodedChunk {
    pub hash: ChunkHash,
    pub raw_len: usize,
    pub codec: Codec,
    pub data: Vec<u8>,
}

impl EncodedChunk {
    const ZSTD_LEVEL: i32 = 3;

    /// Compress when that saves space; otherwise keep the raw bytes.
    pub fn encode(hash: ChunkHash, raw: &[u8]) -> Self {
        match zstd::bulk::compress(raw, Self::ZSTD_LEVEL) {
            Ok(data) if data.len() < raw.len() => Self {
                hash,
                raw_len: raw.len(),
                codec: Codec::Zstd,
                data,
            },
            Ok(_) | Err(_) => Self {
                hash,
                raw_len: raw.len(),
                codec: Codec::Raw,
                data: raw.to_vec(),
            },
        }
    }

    /// The raw bytes of a stored chunk, verified against its hash.
    pub fn decode(hash: ChunkHash, codec: Codec, raw_len: usize, data: Vec<u8>) -> Option<Vec<u8>> {
        let raw = match codec {
            Codec::Raw => data,
            Codec::Zstd => zstd::bulk::decompress(&data, raw_len).ok()?,
        };
        (raw.len() == raw_len && ChunkHash::of(&raw) == hash).then_some(raw)
    }
}

#[cfg(test)]
mod tests {
    use super::{ChunkHash, Codec, EncodedChunk};

    fn pseudo_random(len: usize, seed: u64) -> Vec<u8> {
        let mut state = seed;
        (0..len)
            .map(|_| {
                state = state
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                state.to_be_bytes()[0]
            })
            .collect()
    }

    #[test]
    fn encoding_round_trips_and_detects_corruption() {
        for raw in [vec![b'a'; 40_000], pseudo_random(40_000, 3)] {
            let hash = ChunkHash::of(&raw);
            let chunk = EncodedChunk::encode(hash, &raw);
            if raw[0] == b'a' {
                assert_eq!(chunk.codec, Codec::Zstd);
            }
            let decoded =
                EncodedChunk::decode(hash, chunk.codec, chunk.raw_len, chunk.data.clone());
            assert_eq!(decoded.as_deref(), Some(raw.as_slice()));
            let mut damaged = chunk.data;
            damaged[10] ^= 0xff;
            assert_eq!(
                EncodedChunk::decode(hash, chunk.codec, chunk.raw_len, damaged),
                None
            );
        }
    }
}
