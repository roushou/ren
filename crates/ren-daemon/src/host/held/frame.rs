//! The run host's wire, pure: frames between a daemon and the run host that
//! holds one run's client process, over the host's socket. A frame is a tag
//! byte, its payload's length (four bytes, big-endian), and the payload.
//! The two sides may be different builds of Ren (a run host outlives the
//! daemon that started it), so a frame neither knows is kept, not refused.

use ren_supervisor::ProcessExit;

/// One frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Frame {
    /// Daemon to host, first: send the client's output from this byte of
    /// the spool on.
    Hello { from: u64 },
    /// Daemon to host: bytes for the client's stdin.
    Input(Vec<u8>),
    /// Daemon to host: end the client.
    Kill,
    /// Host to daemon: the client's output, in spool order.
    Output(Vec<u8>),
    /// Host to daemon: the client exited, after all its output.
    Exited(ProcessExit),
    /// A frame of a tag this build does not know, kept as it came.
    Unknown { tag: u8, payload: Vec<u8> },
}

/// Why bytes are not frames.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FrameError {
    #[error("a frame of {0} bytes is past the limit")]
    TooLarge(u32),
    #[error("a hello frame of {0} bytes, not 8")]
    Hello(usize),
    #[error("an exit frame that does not read: {0}")]
    Exit(String),
}

impl Frame {
    /// The most bytes one frame carries.
    pub const MAX: u32 = 1024 * 1024;

    const HELLO: u8 = 1;
    const INPUT: u8 = 2;
    const KILL: u8 = 3;
    const OUTPUT: u8 = 4;
    const EXITED: u8 = 5;

    /// The frame as bytes.
    pub fn encode(&self) -> Vec<u8> {
        let (tag, payload) = match self {
            Self::Hello { from } => (Self::HELLO, from.to_be_bytes().to_vec()),
            Self::Input(bytes) => (Self::INPUT, bytes.clone()),
            Self::Kill => (Self::KILL, Vec::new()),
            Self::Output(bytes) => (Self::OUTPUT, bytes.clone()),
            Self::Exited(exit) => (Self::EXITED, serde_json::to_vec(exit).unwrap_or_default()),
            Self::Unknown { tag, payload } => (*tag, payload.clone()),
        };
        let len = u32::try_from(payload.len()).unwrap_or(u32::MAX);
        let mut bytes = Vec::with_capacity(payload.len() + 5);
        bytes.push(tag);
        bytes.extend(len.to_be_bytes());
        bytes.extend(payload);
        bytes
    }

    fn decode(tag: u8, payload: Vec<u8>) -> Result<Self, FrameError> {
        Ok(match tag {
            Self::HELLO => {
                let bytes: [u8; 8] = payload
                    .as_slice()
                    .try_into()
                    .map_err(|_| FrameError::Hello(payload.len()))?;
                Self::Hello {
                    from: u64::from_be_bytes(bytes),
                }
            }
            Self::INPUT => Self::Input(payload),
            Self::KILL => Self::Kill,
            Self::OUTPUT => Self::Output(payload),
            Self::EXITED => Self::Exited(
                serde_json::from_slice(&payload)
                    .map_err(|error| FrameError::Exit(error.to_string()))?,
            ),
            tag => Self::Unknown { tag, payload },
        })
    }
}

/// Splits arbitrary byte fragments into frames. The frames do not depend on
/// how the bytes were split.
#[derive(Debug, Default)]
pub struct Frames {
    buffer: Vec<u8>,
}

impl Frames {
    /// The frames `bytes` completes.
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<Frame>, FrameError> {
        self.buffer.extend_from_slice(bytes);
        let mut frames = Vec::new();
        while let Some(&[tag, a, b, c, d]) = self.buffer.get(..5) {
            let len = u32::from_be_bytes([a, b, c, d]);
            if len > Frame::MAX {
                return Err(FrameError::TooLarge(len));
            }
            let end = 5 + usize::try_from(len).map_err(|_| FrameError::TooLarge(len))?;
            if self.buffer.len() < end {
                break;
            }
            let payload = self.buffer.get(5..end).unwrap_or_default().to_vec();
            self.buffer.drain(..end);
            frames.push(Frame::decode(tag, payload)?);
        }
        Ok(frames)
    }
}

#[cfg(test)]
mod tests {
    use super::{Frame, FrameError, Frames};
    use ren_supervisor::ProcessExit;

    fn sample() -> Vec<Frame> {
        vec![
            Frame::Hello { from: 1 << 40 },
            Frame::Input(b"{\"id\":1}\n".to_vec()),
            Frame::Kill,
            Frame::Output(Vec::new()),
            Frame::Output(vec![7; 70_000]),
            Frame::Exited(ProcessExit::Signal(9)),
            Frame::Exited(ProcessExit::Code(0)),
            Frame::Unknown {
                tag: 200,
                payload: b"later".to_vec(),
            },
        ]
    }

    #[test]
    fn frames_read_back_however_the_bytes_are_split() {
        let bytes: Vec<u8> = sample().iter().flat_map(Frame::encode).collect();
        for piece in [1, 2, 3, 5, 7, 4096, bytes.len()] {
            let mut frames = Frames::default();
            let mut read = Vec::new();
            for chunk in bytes.chunks(piece) {
                read.extend(frames.push(chunk).unwrap());
            }
            assert_eq!(read, sample(), "pieces of {piece}");
        }
    }

    #[test]
    fn a_frame_too_large_or_misshapen_is_refused() {
        let mut too_large = vec![4];
        too_large.extend((Frame::MAX + 1).to_be_bytes());
        assert_eq!(
            Frames::default().push(&too_large),
            Err(FrameError::TooLarge(Frame::MAX + 1))
        );
        let short_hello = [1, 0, 0, 0, 2, 0, 0];
        assert_eq!(
            Frames::default().push(&short_hello),
            Err(FrameError::Hello(2))
        );
        let bad_exit = Frame::Unknown {
            tag: 5,
            payload: b"nope".to_vec(),
        }
        .encode();
        assert!(matches!(
            Frames::default().push(&bad_exit),
            Err(FrameError::Exit(_))
        ));
    }
}
