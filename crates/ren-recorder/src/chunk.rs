//! Content-defined chunking of bodies.
//!
//! Bodies are cut with FastCDC, so a byte range repeated across bodies (a
//! conversation prefix resent every turn) produces the same chunks and is
//! stored once. Cutting is sans-IO: [`Chunker`] consumes bytes and emits cuts.

use bytes::Bytes;
use fastcdc::v2020::FastCDC;

/// Chunk size bounds: 16 KiB on average, never above 64 KiB.
const MIN_CHUNK: usize = 8 * 1024;
const AVG_CHUNK: usize = 16 * 1024;
pub const MAX_CHUNK: usize = 64 * 1024;

/// One cut: the bytes of a chunk and where they start in the body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cut {
    pub offset: u64,
    pub bytes: Bytes,
}

/// Cuts one contiguous byte range of a body into content-defined chunks.
///
/// Every chunk except the last depends only on bytes the chunker has seen, so
/// it is emitted as soon as the buffer holds enough to decide. The tail stays
/// buffered (at most two maximum chunks) until more bytes or [`Chunker::flush`].
#[derive(Debug)]
pub struct Chunker {
    start: u64,
    buffer: Vec<u8>,
}

impl Chunker {
    /// A chunker whose first byte is at `offset` in the body.
    pub fn at(offset: u64) -> Self {
        Self {
            start: offset,
            buffer: Vec::new(),
        }
    }

    /// The body offset the next pushed byte must have to be contiguous.
    pub fn next_offset(&self) -> u64 {
        self.start
            .saturating_add(u64::try_from(self.buffer.len()).unwrap_or(u64::MAX))
    }

    pub fn push(&mut self, bytes: &[u8], cuts: &mut Vec<Cut>) {
        self.buffer.extend_from_slice(bytes);
        if self.buffer.len() >= 2 * MAX_CHUNK {
            self.cut(Tail::Keep, cuts);
        }
    }

    /// Emit everything buffered, including the final partial chunk.
    pub fn flush(&mut self, cuts: &mut Vec<Cut>) {
        self.cut(Tail::Emit, cuts);
    }

    fn cut(&mut self, tail: Tail, cuts: &mut Vec<Cut>) {
        if self.buffer.is_empty() {
            return;
        }
        let mut chunks: Vec<_> = FastCDC::new(&self.buffer, MIN_CHUNK, AVG_CHUNK, MAX_CHUNK)
            .map(|chunk| (chunk.offset, chunk.length))
            .collect();
        let kept = match tail {
            Tail::Emit => None,
            Tail::Keep => chunks.pop(),
        };
        for (offset, length) in chunks {
            let Some(bytes) = self.buffer.get(offset..offset.saturating_add(length)) else {
                continue;
            };
            cuts.push(Cut {
                offset: self
                    .start
                    .saturating_add(u64::try_from(offset).unwrap_or(u64::MAX)),
                bytes: Bytes::copy_from_slice(bytes),
            });
        }
        let consumed = kept.map_or(self.buffer.len(), |(offset, _)| offset);
        self.buffer.drain(..consumed);
        self.start = self
            .start
            .saturating_add(u64::try_from(consumed).unwrap_or(u64::MAX));
    }
}

#[derive(Debug, Clone, Copy)]
enum Tail {
    Keep,
    Emit,
}

#[cfg(test)]
mod tests {
    use super::{Chunker, Cut, MAX_CHUNK};
    use proptest::prelude::*;

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

    fn cut_all(body: &[u8], split: &[usize]) -> Vec<Cut> {
        let mut chunker = Chunker::at(0);
        let mut cuts = Vec::new();
        let mut rest = body;
        for &at in split {
            let (head, tail) = rest.split_at(at.min(rest.len()));
            chunker.push(head, &mut cuts);
            rest = tail;
        }
        chunker.push(rest, &mut cuts);
        chunker.flush(&mut cuts);
        cuts
    }

    #[test]
    fn cuts_are_contiguous_bounded_and_reassemble_the_body() {
        let body = pseudo_random(700_000, 7);
        let cuts = cut_all(&body, &[]);
        let mut next = 0;
        let mut joined = Vec::new();
        for cut in &cuts {
            assert_eq!(cut.offset, next);
            assert!(cut.bytes.len() <= MAX_CHUNK);
            next += u64::try_from(cut.bytes.len()).unwrap();
            joined.extend_from_slice(&cut.bytes);
        }
        assert_eq!(joined, body);
        assert!(cuts.len() > 10);
    }

    #[test]
    fn a_shared_prefix_produces_shared_chunks() {
        let prefix = pseudo_random(400_000, 1);
        let mut longer = prefix.clone();
        longer.extend(pseudo_random(100_000, 2));
        let first: Vec<_> = cut_all(&prefix, &[])
            .into_iter()
            .map(|c| c.bytes.clone())
            .collect();
        let second: Vec<_> = cut_all(&longer, &[])
            .into_iter()
            .map(|c| c.bytes.clone())
            .collect();
        let shared = first.iter().filter(|hash| second.contains(hash)).count();
        // Only the chunk that ended at the old body's end can differ.
        assert!(shared + 1 >= first.len(), "{shared} of {}", first.len());
    }

    proptest! {
        #[test]
        fn cuts_do_not_depend_on_how_bytes_arrive(
            seed in any::<u64>(),
            len in 0usize..400_000,
            split in proptest::collection::vec(0usize..100_000, 0..8),
        ) {
            let body = pseudo_random(len, seed);
            prop_assert_eq!(cut_all(&body, &split), cut_all(&body, &[]));
        }
    }
}
