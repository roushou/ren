//! A prompt to a driven client: its text and the images sent with it, never
//! blank. Each client's codec turns it into its client's input.

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

/// A prompt: its text, and the images sent with it. Never blank: it has
/// text, images, or both.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prompt {
    text: String,
    images: Vec<PromptImage>,
}

/// Sent as its text, or, with images, as its text and what each image is
/// (never their bytes).
impl serde::Serialize for Prompt {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        if self.images.is_empty() {
            return serializer.serialize_str(&self.text);
        }
        let mut prompt = serializer.serialize_struct("Prompt", 2)?;
        prompt.serialize_field("text", &self.text)?;
        let images: Vec<&Attachment> = self.images.iter().map(PromptImage::read).collect();
        prompt.serialize_field("images", &images)?;
        prompt.end()
    }
}

/// Read back as text that is not blank. A prompt with images is not read
/// back: their bytes were never written.
impl<'de> serde::Deserialize<'de> for Prompt {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::parse(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("a prompt cannot be blank")]
pub struct BlankPrompt;

impl Prompt {
    pub fn parse(text: String) -> Result<Self, BlankPrompt> {
        Self::with_images(text, Vec::new())
    }

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
    use super::{BlankPrompt, Prompt, PromptImage};

    #[test]
    fn a_prompt_with_images_shows_what_they_are_never_their_bytes() {
        assert_eq!(Prompt::parse("  ".into()), Err(BlankPrompt));
        let images = Prompt::sample("");
        assert_eq!(images.as_str(), "");
        let text = serde_json::to_value(Prompt::parse("go".into()).unwrap()).unwrap();
        assert_eq!(text, "go");
        let shown = serde_json::to_value(Prompt::sample("look")).unwrap();
        assert_eq!(shown["text"], "look");
        assert_eq!(shown["images"][0]["name"], "shot.png");
        assert!(!shown.to_string().contains(&PromptImage::sample().base64()));
        assert!(serde_json::from_value::<Prompt>(shown).is_err());
        assert!(!format!("{:?}", PromptImage::sample()).contains("bytes"));
    }
}
