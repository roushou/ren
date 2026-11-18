//! A prompt to a driven client, as it is sent (`Prompt`: its text and the
//! images sent with it, bytes and all) and as Ren records it
//! (`RecordedPrompt`: its text and what each image is). Never blank. Each
//! client's codec turns a sent prompt into its client's input.

use crate::attachment::{Attachment, AttachmentKind};
use base64::Engine as _;
use std::path::PathBuf;
use std::sync::Arc;

/// An image sent with a prompt: what it is, where its file is, and its
/// bytes. Each client's codec takes the one its client reads: Codex the
/// path, the others the bytes, in base64.
#[derive(Clone, PartialEq, Eq)]
pub struct PromptImage {
    read: Attachment,
    path: PathBuf,
    bytes: Arc<[u8]>,
}

impl PromptImage {
    /// `read`, kept at `path`, whose bytes are `bytes`.
    pub fn new(read: Attachment, path: PathBuf, bytes: Arc<[u8]>) -> Self {
        Self { read, path, bytes }
    }

    pub fn read(&self) -> &Attachment {
        &self.read
    }

    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    /// Its format's media type.
    pub fn media_type(&self) -> &'static str {
        match self.read.kind() {
            AttachmentKind::Image(image) => image.format().media_type(),
        }
    }

    /// Its bytes in base64, as the inline clients take them.
    pub fn base64(&self) -> String {
        base64::engine::general_purpose::STANDARD.encode(&self.bytes)
    }
}

#[cfg(test)]
impl PromptImage {
    /// A 2×1 PNG header, `shot.png`, kept at `/data/attachments/s/a/shot.png`.
    pub(crate) fn sample() -> Self {
        let mut bytes = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
        bytes.extend(2u32.to_be_bytes());
        bytes.extend(1u32.to_be_bytes());
        bytes.extend([8, 6, 0, 0, 0, 0, 0, 0, 0]);
        let read =
            Attachment::parse("shot.png", &bytes, &crate::attachment::Limits::default()).unwrap();
        Self::new(read, "/data/attachments/s/a/shot.png".into(), bytes.into())
    }
}

/// What it is and where, never its bytes.
impl std::fmt::Debug for PromptImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PromptImage")
            .field("read", &self.read)
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

/// A prompt as it is sent: its text, and the images sent with it, bytes
/// and all. Never blank: it has text, images, or both. It is never read
/// back; what Ren writes of it is its [`RecordedPrompt`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prompt {
    text: String,
    images: Vec<PromptImage>,
}

/// Written as its record, never with its images' bytes.
impl serde::Serialize for Prompt {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.record().serialize(serializer)
    }
}

/// A prompt as Ren records and shows it: its text, and what each image
/// sent with it is, never their bytes. Never blank, like the prompt it
/// records, and read back as it was written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedPrompt {
    text: String,
    images: Vec<Attachment>,
}

/// Its text alone, or, with images, `{text, images}`.
impl serde::Serialize for RecordedPrompt {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        if self.images.is_empty() {
            return serializer.serialize_str(&self.text);
        }
        let mut prompt = serializer.serialize_struct("RecordedPrompt", 2)?;
        prompt.serialize_field("text", &self.text)?;
        prompt.serialize_field("images", &self.images)?;
        prompt.end()
    }
}

/// Either shape it is written in, refused when blank.
impl<'de> serde::Deserialize<'de> for RecordedPrompt {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let (text, images) = match Written::deserialize(deserializer)? {
            Written::Text(text) => (text, Vec::new()),
            Written::WithImages { text, images } => (text, images),
        };
        Self::new(text, images).map_err(serde::de::Error::custom)
    }
}

/// The shapes a [`RecordedPrompt`] is written in.
#[derive(serde::Deserialize)]
#[serde(untagged)]
enum Written {
    Text(String),
    WithImages {
        text: String,
        images: Vec<Attachment>,
    },
}

impl RecordedPrompt {
    /// `text` with what each of `images` is; refused when both are empty.
    fn new(text: String, images: Vec<Attachment>) -> Result<Self, BlankPrompt> {
        if text.trim().is_empty() && images.is_empty() {
            Err(BlankPrompt)
        } else {
            Ok(Self { text, images })
        }
    }

    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// What each image sent with it is.
    pub fn images(&self) -> &[Attachment] {
        &self.images
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("a prompt cannot be blank")]
pub struct BlankPrompt;

impl TryFrom<String> for Prompt {
    type Error = BlankPrompt;

    /// Text alone, refused when blank.
    fn try_from(text: String) -> Result<Self, Self::Error> {
        Self::with_images(text, Vec::new())
    }
}

impl Prompt {
    /// `text` with `images`; the text may be blank when images go.
    pub fn with_images(text: String, images: Vec<PromptImage>) -> Result<Self, BlankPrompt> {
        if text.trim().is_empty() && images.is_empty() {
            Err(BlankPrompt)
        } else {
            Ok(Self { text, images })
        }
    }

    pub fn as_str(&self) -> &str {
        &self.text
    }

    pub fn images(&self) -> &[PromptImage] {
        &self.images
    }

    /// What each image sent with it is, for the echo of this prompt.
    pub fn sent(&self) -> Vec<Attachment> {
        self.images.iter().map(|image| image.read.clone()).collect()
    }

    /// What Ren records of it: its text, and what each image is.
    pub fn record(&self) -> RecordedPrompt {
        RecordedPrompt {
            text: self.text.clone(),
            images: self.sent(),
        }
    }
}

#[cfg(test)]
impl Prompt {
    /// A prompt of `text` with the sample image.
    pub(crate) fn sample(text: &str) -> Self {
        Self::with_images(text.into(), vec![PromptImage::sample()]).unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::{BlankPrompt, Prompt, PromptImage, RecordedPrompt};

    #[test]
    fn a_prompt_with_images_shows_what_they_are_never_their_bytes() {
        assert_eq!(Prompt::try_from("  ".to_owned()), Err(BlankPrompt));
        let images = Prompt::sample("");
        assert_eq!(images.as_str(), "");
        let text = serde_json::to_value(Prompt::try_from("go".to_owned()).unwrap()).unwrap();
        assert_eq!(text, "go");
        let shown = serde_json::to_value(Prompt::sample("look")).unwrap();
        assert_eq!(shown["text"], "look");
        assert_eq!(shown["images"][0]["name"], "shot.png");
        assert!(!shown.to_string().contains(&PromptImage::sample().base64()));
        assert!(!format!("{:?}", PromptImage::sample()).contains("bytes"));
    }

    /// What is written of a sent prompt reads back as its record, in either
    /// shape; a blank one is refused.
    #[test]
    fn a_recorded_prompt_reads_back_as_it_was_written() {
        for sent in [
            Prompt::try_from("go".to_owned()).unwrap(),
            Prompt::sample("look"),
        ] {
            let written = serde_json::to_value(&sent).unwrap();
            let read: RecordedPrompt = serde_json::from_value(written.clone()).unwrap();
            assert_eq!(read, sent.record());
            assert_eq!(read.as_str(), sent.as_str());
            assert_eq!(read.images(), sent.sent());
            assert_eq!(serde_json::to_value(&read).unwrap(), written);
        }
        for blank in [
            serde_json::json!(" "),
            serde_json::json!({ "text": "", "images": [] }),
        ] {
            assert!(serde_json::from_value::<RecordedPrompt>(blank).is_err());
        }
    }
}
