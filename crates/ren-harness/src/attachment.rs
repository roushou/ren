//! Attachments to a prompt: what Ren takes, read once from the bytes an
//! operator uploaded, and how each client takes them. Pure: the bytes are
//! handed in, and nothing here reads or writes a file.
//!
//! An [`Attachment`] is known to be what it says: [`Attachment::parse`]
//! reads its format from its leading bytes (never its name or a media type
//! someone claimed) and its dimensions from its header, within [`Limits`].
//! Each client declares, as data, which kinds it takes and how
//! ([`Harness::acceptance`]); only its codec turns an attachment into the
//! client's own input.

use crate::Harness;
use ren_depot::source::Sha256;
use serde_with::{DeserializeFromStr, SerializeDisplay};
use sha2::Digest;
use std::fmt;
use strum::{Display, EnumString, IntoStaticStr, VariantArray};

/// An image format Ren sends to clients.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum ImageFormat {
    Png,
    Jpeg,
    Gif,
    Webp,
}

impl ImageFormat {
    /// Its media type, as the clients and provider APIs name it.
    pub fn media_type(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::Gif => "image/gif",
            Self::Webp => "image/webp",
        }
    }

    /// The format the leading bytes announce, if Ren takes it.
    fn sniff(bytes: &[u8]) -> Option<Self> {
        let at = |range: std::ops::Range<usize>, magic: &[u8]| bytes.get(range) == Some(magic);
        if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
            Some(Self::Png)
        } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
            Some(Self::Jpeg)
        } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
            Some(Self::Gif)
        } else if at(0..4, b"RIFF") && at(8..12, b"WEBP") {
            Some(Self::Webp)
        } else {
            None
        }
    }
}

/// A count of bytes.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(transparent)]
pub struct ByteSize(u64);

impl ByteSize {
    pub const fn new(bytes: u64) -> Self {
        Self(bytes)
    }

    /// `n` mebibytes.
    pub const fn mebibytes(n: u64) -> Self {
        Self(n.saturating_mul(1024 * 1024))
    }

    pub fn get(self) -> u64 {
        self.0
    }

    /// These bytes once encoded in base64, padded: four characters for
    /// every three bytes or part of three.
    pub fn base64(self) -> Self {
        Self(self.0.div_ceil(3).saturating_mul(4))
    }

    fn of(bytes: &[u8]) -> Self {
        Self(u64::try_from(bytes.len()).unwrap_or(u64::MAX))
    }
}

impl fmt::Display for ByteSize {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} bytes", self.0)
    }
}

/// An image's size in pixels: never zero on either side.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(try_from = "RawDimensions")]
pub struct Dimensions {
    width: u32,
    height: u32,
}

impl Dimensions {
    /// `None` with a side of zero.
    pub fn new(width: u32, height: u32) -> Option<Self> {
        (width > 0 && height > 0).then_some(Self { width, height })
    }

    pub fn width(self) -> u32 {
        self.width
    }

    pub fn height(self) -> u32 {
        self.height
    }

    /// The longer of its two sides.
    pub fn longest_side(self) -> u32 {
        self.width.max(self.height)
    }

    /// Read from the image's header; `None` for one that has no size Ren
    /// can read, or a side of zero or beyond `u32`.
    fn read(bytes: &[u8]) -> Option<Self> {
        let size = imagesize::blob_size(bytes).ok()?;
        Self::new(
            u32::try_from(size.width).ok()?,
            u32::try_from(size.height).ok()?,
        )
    }
}

/// Dimensions as written, read back only through [`Dimensions::new`].
#[derive(serde::Deserialize)]
struct RawDimensions {
    width: u32,
    height: u32,
}

impl TryFrom<RawDimensions> for Dimensions {
    type Error = &'static str;

    fn try_from(raw: RawDimensions) -> Result<Self, Self::Error> {
        Self::new(raw.width, raw.height).ok_or("an image has no side of zero")
    }
}

impl fmt::Display for Dimensions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}×{}", self.width, self.height)
    }
}

/// An image Ren read: its format and its size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct Image {
    format: ImageFormat,
    dimensions: Dimensions,
}

impl Image {
    /// An image read before, as stored.
    pub fn new(format: ImageFormat, dimensions: Dimensions) -> Self {
        Self { format, dimensions }
    }

    pub fn format(self) -> ImageFormat {
        self.format
    }

    pub fn dimensions(self) -> Dimensions {
        self.dimensions
    }
}

/// What an attachment is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AttachmentKind {
    Image(Image),
}

/// An attachment's name, for showing it: the last part of the name it was
/// uploaded with, without control characters. Never a path, never `.` or
/// `..`, never empty, and at most [`AttachmentName::MAX_BYTES`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(transparent)]
pub struct AttachmentName(String);

impl AttachmentName {
    /// The most a file name takes on common filesystems.
    pub const MAX_BYTES: usize = 255;

    pub fn parse(uploaded: &str) -> Result<Self, NameError> {
        let last = uploaded.rsplit(['/', '\\']).next().unwrap_or_default();
        let name: String = last.chars().filter(|c| !c.is_control()).collect();
        let name = name.trim();
        if name.is_empty() {
            return Err(NameError::Empty);
        }
        if name == "." || name == ".." {
            return Err(NameError::Reserved);
        }
        if name.len() > Self::MAX_BYTES {
            return Err(NameError::TooLong);
        }
        Ok(Self(name.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for AttachmentName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Why a name is not an attachment's name. The name is not echoed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum NameError {
    #[error("an attachment needs a name")]
    Empty,
    #[error("an attachment cannot be named `.` or `..`")]
    Reserved,
    #[error(
        "an attachment's name is longer than {} bytes",
        AttachmentName::MAX_BYTES
    )]
    TooLong,
}

/// How much Ren takes: for each image, and for each prompt's attachments
/// together. The defaults follow the Anthropic API's image limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// The most bytes one image has, as uploaded.
    image_bytes: ByteSize,
    /// The longest side, in pixels, of one image.
    image_side: u32,
    /// The most attachments in one prompt.
    prompt_count: usize,
    /// The most bytes one prompt's attachments put on the wire, encoded.
    prompt_bytes: ByteSize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            image_bytes: ByteSize::mebibytes(5),
            image_side: 8000,
            prompt_count: 10,
            prompt_bytes: ByteSize::mebibytes(20),
        }
    }
}

impl Limits {
    /// The most bytes one image has, as uploaded: an upload's cap.
    pub fn image_bytes(&self) -> ByteSize {
        self.image_bytes
    }

    /// Whether `attachments` can go in one prompt together. Each was read
    /// within these limits; this checks how many there are, and the bytes
    /// they make in base64, which is how inline clients send them.
    pub fn check_prompt(&self, attachments: &[Attachment]) -> Result<(), PromptLimitError> {
        if attachments.len() > self.prompt_count {
            return Err(PromptLimitError::TooMany {
                count: attachments.len(),
                limit: self.prompt_count,
            });
        }
        let encoded = ByteSize(
            attachments
                .iter()
                .map(|attachment| attachment.size.base64().get())
                .fold(0, u64::saturating_add),
        );
        if encoded > self.prompt_bytes {
            return Err(PromptLimitError::TooLarge {
                size: encoded,
                limit: self.prompt_bytes,
            });
        }
        Ok(())
    }
}

/// The `[attachments]` configuration section: [`Limits`] as written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AttachmentOptions {
    /// The most bytes one image has, as uploaded.
    pub image_bytes: u64,
    /// The longest side, in pixels, of one image.
    pub image_side: u32,
    /// The most attachments in one prompt.
    pub prompt_count: usize,
    /// The most bytes one prompt's attachments come to in base64.
    pub prompt_bytes: u64,
}

impl Default for AttachmentOptions {
    fn default() -> Self {
        let limits = Limits::default();
        Self {
            image_bytes: limits.image_bytes.get(),
            image_side: limits.image_side,
            prompt_count: limits.prompt_count,
            prompt_bytes: limits.prompt_bytes.get(),
        }
    }
}

impl AttachmentOptions {
    /// The limits these options set: none zero, and one image fitting in a
    /// prompt.
    pub fn limits(&self) -> Result<Limits, AttachmentOptionsError> {
        if self.image_bytes == 0
            || self.image_side == 0
            || self.prompt_count == 0
            || self.prompt_bytes == 0
        {
            return Err(AttachmentOptionsError::Zero);
        }
        let image_bytes = ByteSize(self.image_bytes);
        let prompt_bytes = ByteSize(self.prompt_bytes);
        if image_bytes.base64() > prompt_bytes {
            return Err(AttachmentOptionsError::ImageBeyondPrompt);
        }
        Ok(Limits {
            image_bytes,
            image_side: self.image_side,
            prompt_count: self.prompt_count,
            prompt_bytes,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AttachmentOptionsError {
    #[error("attachments limits must not be zero")]
    Zero,
    #[error("attachments.prompt_bytes must hold one image of attachments.image_bytes in base64")]
    ImageBeyondPrompt,
}

/// Why a prompt's attachments cannot go together.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PromptLimitError {
    #[error("a prompt takes at most {limit} attachments, not {count}")]
    TooMany { count: usize, limit: usize },
    #[error("a prompt's attachments come to {size} encoded, more than {limit}")]
    TooLarge { size: ByteSize, limit: ByteSize },
}

/// One file uploaded to go with a prompt, read and known to be what it
/// says. Ren's identity for it is the store's; this is what it is. Read
/// back from JSON through [`Attachment::restore`].
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(try_from = "StoredAttachment")]
pub struct Attachment {
    name: AttachmentName,
    kind: AttachmentKind,
    size: ByteSize,
    sha256: Sha256,
}

impl Attachment {
    /// Read the bytes uploaded as `name`, within `limits`. Ren takes images
    /// in the formats of [`ImageFormat`], told by their leading bytes.
    pub fn parse(name: &str, bytes: &[u8], limits: &Limits) -> Result<Self, AttachmentError> {
        let name = AttachmentName::parse(name)?;
        if bytes.is_empty() {
            return Err(AttachmentError::Empty);
        }
        let format = ImageFormat::sniff(bytes).ok_or(AttachmentError::Unrecognized)?;
        let size = ByteSize::of(bytes);
        if size > limits.image_bytes {
            return Err(AttachmentError::TooLarge {
                size,
                limit: limits.image_bytes,
            });
        }
        let dimensions = Dimensions::read(bytes).ok_or(AttachmentError::Unreadable(format))?;
        if dimensions.longest_side() > limits.image_side {
            return Err(AttachmentError::TooWide {
                dimensions,
                limit: limits.image_side,
            });
        }
        Ok(Self {
            name,
            kind: AttachmentKind::Image(Image { format, dimensions }),
            size,
            sha256: Sha256::from_bytes(sha2::Sha256::digest(bytes).into()),
        })
    }

    /// An attachment read before, as stored: its name is read again, so a
    /// stored name that is not one is refused.
    pub fn restore(
        name: &str,
        kind: AttachmentKind,
        size: ByteSize,
        sha256: Sha256,
    ) -> Result<Self, NameError> {
        Ok(Self {
            name: AttachmentName::parse(name)?,
            kind,
            size,
            sha256,
        })
    }

    pub fn name(&self) -> &AttachmentName {
        &self.name
    }

    pub fn kind(&self) -> AttachmentKind {
        self.kind
    }

    pub fn size(&self) -> ByteSize {
        self.size
    }

    pub fn sha256(&self) -> Sha256 {
        self.sha256
    }
}

/// An attachment as written.
#[derive(serde::Deserialize)]
struct StoredAttachment {
    name: String,
    kind: AttachmentKind,
    size: ByteSize,
    sha256: Sha256,
}

impl TryFrom<StoredAttachment> for Attachment {
    type Error = NameError;

    fn try_from(stored: StoredAttachment) -> Result<Self, Self::Error> {
        Self::restore(&stored.name, stored.kind, stored.size, stored.sha256)
    }
}

/// Why uploaded bytes are not an attachment Ren takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AttachmentError {
    #[error(transparent)]
    Name(#[from] NameError),
    #[error("the attachment is empty")]
    Empty,
    #[error("the attachment is not a PNG, JPEG, GIF, or WebP image")]
    Unrecognized,
    #[error("the {0} image's size cannot be read")]
    Unreadable(ImageFormat),
    #[error("the image is {size}, more than {limit}")]
    TooLarge { size: ByteSize, limit: ByteSize },
    #[error("the image is {dimensions} pixels, longer than {limit} on a side")]
    TooWide { dimensions: Dimensions, limit: u32 },
}

/// How a client takes an attachment.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum Delivery {
    /// Its bytes, in base64, inside the message.
    InlineBase64,
    /// The path of its file, in an input item of the client's own.
    LocalPath,
}

/// Whether a client takes a kind of attachment, and how.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Takes {
    /// Always, this way.
    Always(Delivery),
    /// This way, when the running client says it does.
    IfOffered(Delivery),
    /// Never.
    Never,
}

/// What the running client said of a kind of attachment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Offer {
    Offered,
    NotOffered,
    /// It has not said, or no process is running to say.
    Unknown,
}

/// Why a client cannot be sent an attachment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum NotAccepted {
    #[error("this client takes no attachments of this kind")]
    Never,
    #[error("this client said it takes no attachments of this kind")]
    NotOffered,
    #[error("this client has not said whether it takes attachments of this kind")]
    Unknown,
}

impl Takes {
    /// How to send the attachment, given what the running client said. A
    /// client that has not said is not taken to accept.
    pub fn resolve(self, offer: Offer) -> Result<Delivery, NotAccepted> {
        match (self, offer) {
            (Self::Always(delivery), Offer::Offered | Offer::NotOffered | Offer::Unknown)
            | (Self::IfOffered(delivery), Offer::Offered) => Ok(delivery),
            (Self::IfOffered(_), Offer::NotOffered) => Err(NotAccepted::NotOffered),
            (Self::IfOffered(_), Offer::Unknown) => Err(NotAccepted::Unknown),
            (Self::Never, Offer::Offered | Offer::NotOffered | Offer::Unknown) => {
                Err(NotAccepted::Never)
            }
        }
    }
}

/// What a client takes, by kind of attachment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Acceptance {
    images: Takes,
}

impl Acceptance {
    /// Whether, and how, the client takes images.
    pub fn images(self) -> Takes {
        self.images
    }

    /// Whether, and how, the client takes `kind`.
    pub fn of(self, kind: AttachmentKind) -> Takes {
        match kind {
            AttachmentKind::Image(_) => self.images,
        }
    }
}

impl Harness {
    /// What this client takes in its structured mode, as Ren drives it.
    /// Codex reads an image from its path (`localImage` in `turn/start` and
    /// `turn/steer`); Claude Code and Pi take it in base64 in the message
    /// (an `image` content block; `images` on `prompt` and `steer`);
    /// OpenCode takes an ACP `image` block when its `initialize` answer
    /// says `promptCapabilities.image`.
    pub fn acceptance(self) -> Acceptance {
        let images = match self {
            Self::Codex => Takes::Always(Delivery::LocalPath),
            Self::ClaudeCode | Self::Pi => Takes::Always(Delivery::InlineBase64),
            Self::OpenCode => Takes::IfOffered(Delivery::InlineBase64),
        };
        Acceptance { images }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Attachment, AttachmentError, AttachmentKind, AttachmentName, ByteSize, Delivery,
        Dimensions, Image, ImageFormat, Limits, NameError, NotAccepted, Offer, PromptLimitError,
        Takes,
    };
    use crate::Harness;
    use proptest::prelude::*;

    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
        bytes.extend(width.to_be_bytes());
        bytes.extend(height.to_be_bytes());
        bytes.extend([8, 6, 0, 0, 0, 0, 0, 0, 0]);
        bytes
    }

    fn jpeg(width: u16, height: u16) -> Vec<u8> {
        let mut bytes = vec![0xFF, 0xD8, 0xFF, 0xC0, 0x00, 0x11, 0x08];
        bytes.extend(height.to_be_bytes());
        bytes.extend(width.to_be_bytes());
        bytes.extend([0x03, 1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1]);
        bytes
    }

    fn gif(width: u16, height: u16) -> Vec<u8> {
        let mut bytes = b"GIF89a".to_vec();
        bytes.extend(width.to_le_bytes());
        bytes.extend(height.to_le_bytes());
        bytes.extend([0, 0, 0]);
        bytes
    }

    /// An extended WebP: its sides less one, in 24 bits each.
    fn webp(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = b"RIFF\x16\0\0\0WEBPVP8X\x0a\0\0\0\0\0\0\0".to_vec();
        bytes.extend(&(width - 1).to_le_bytes()[..3]);
        bytes.extend(&(height - 1).to_le_bytes()[..3]);
        bytes
    }

    fn image(attachment: &Attachment) -> Image {
        match attachment.kind() {
            AttachmentKind::Image(image) => image,
        }
    }

    #[test]
    fn reads_each_format_from_its_bytes_not_its_name() {
        let limits = Limits::default();
        for (bytes, format) in [
            (png(640, 480), ImageFormat::Png),
            (jpeg(640, 480), ImageFormat::Jpeg),
            (gif(640, 480), ImageFormat::Gif),
            (webp(640, 480), ImageFormat::Webp),
        ] {
            let read = Attachment::parse("screenshot.txt", &bytes, &limits).unwrap();
            let image = image(&read);
            assert_eq!(image.format(), format);
            assert_eq!(
                (image.dimensions().width(), image.dimensions().height()),
                (640, 480),
                "{format}"
            );
            assert_eq!(read.size(), ByteSize::new(bytes.len().try_into().unwrap()));
        }
        assert_eq!(ImageFormat::Webp.media_type(), "image/webp");
    }

    #[test]
    fn refuses_what_is_not_an_image_ren_takes() {
        let limits = Limits::default();
        let refused = |bytes: &[u8]| Attachment::parse("a.png", bytes, &limits).unwrap_err();
        assert_eq!(refused(b""), AttachmentError::Empty);
        assert_eq!(
            refused(b"<svg xmlns='http://www.w3.org/2000/svg'/>"),
            AttachmentError::Unrecognized
        );
        assert_eq!(refused(b"%PDF-1.7"), AttachmentError::Unrecognized);
        // The format's magic, without a size to read.
        assert_eq!(
            refused(b"\x89PNG\r\n\x1a\n"),
            AttachmentError::Unreadable(ImageFormat::Png)
        );
        assert_eq!(
            refused(&png(0, 480)),
            AttachmentError::Unreadable(ImageFormat::Png)
        );
    }

    #[test]
    fn holds_each_image_and_prompt_to_its_limits() {
        let limits = Limits::default();
        assert_eq!(
            Attachment::parse("wide.png", &png(8001, 10), &limits).unwrap_err(),
            AttachmentError::TooWide {
                dimensions: Dimensions {
                    width: 8001,
                    height: 10
                },
                limit: 8000
            }
        );
        let mut large = png(100, 100);
        large.resize(5 * 1024 * 1024 + 1, 0);
        assert!(matches!(
            Attachment::parse("large.png", &large, &limits),
            Err(AttachmentError::TooLarge { .. })
        ));

        let one = Attachment::parse("a.png", &png(10, 10), &limits).unwrap();
        assert_eq!(limits.check_prompt(&vec![one.clone(); 10]), Ok(()));
        assert_eq!(
            limits.check_prompt(&vec![one; 11]),
            Err(PromptLimitError::TooMany {
                count: 11,
                limit: 10
            })
        );
        // Four images of 5 MiB fit in 20 MiB as uploaded, not in base64.
        large.truncate(5 * 1024 * 1024);
        let big = Attachment::parse("big.png", &large, &limits).unwrap();
        assert!(matches!(
            limits.check_prompt(&vec![big; 4]),
            Err(PromptLimitError::TooLarge { .. })
        ));
    }

    #[test]
    fn names_are_the_last_part_and_never_a_path() {
        let name = |raw: &str| AttachmentName::parse(raw).map(|name| name.as_str().to_owned());
        assert_eq!(name("shot.png").as_deref(), Ok("shot.png"));
        assert_eq!(name("../../etc/passwd").as_deref(), Ok("passwd"));
        assert_eq!(name("C:\\Users\\x\\shot.png").as_deref(), Ok("shot.png"));
        assert_eq!(name(" a\u{0}\nb.png ").as_deref(), Ok("ab.png"));
        assert_eq!(name("dir/"), Err(NameError::Empty));
        assert_eq!(name("a/.."), Err(NameError::Reserved));
        assert_eq!(name(&"a".repeat(256)), Err(NameError::TooLong));
    }

    #[test]
    fn options_set_the_limits_and_refuse_ones_that_cannot_hold() {
        use super::{AttachmentOptions, AttachmentOptionsError};
        let options = AttachmentOptions::default();
        assert_eq!(options.limits(), Ok(Limits::default()));
        let parsed: AttachmentOptions = toml::from_str("image_side = 2000").unwrap();
        let limits = parsed.limits().unwrap();
        assert!(matches!(
            Attachment::parse("a.png", &png(2001, 1), &limits),
            Err(AttachmentError::TooWide { .. })
        ));
        let zero = AttachmentOptions {
            prompt_count: 0,
            ..options
        };
        assert_eq!(zero.limits(), Err(AttachmentOptionsError::Zero));
        let tight = AttachmentOptions {
            prompt_bytes: options.image_bytes,
            ..options
        };
        assert_eq!(
            tight.limits(),
            Err(AttachmentOptionsError::ImageBeyondPrompt)
        );
        assert!(toml::from_str::<AttachmentOptions>("images = 1").is_err());
    }

    #[test]
    fn an_attachment_serializes_what_it_is_and_restores_from_it() {
        let read = Attachment::parse("dir/shot.png", &png(3, 2), &Limits::default()).unwrap();
        let json = serde_json::to_value(&read).unwrap();
        assert_eq!(json["name"], "shot.png");
        assert_eq!(json["kind"]["kind"], "image");
        assert_eq!(json["kind"]["format"], "png");
        assert_eq!(json["kind"]["dimensions"]["width"], 3);
        assert_eq!(json["size"], read.size().get());
        assert_eq!(json["sha256"], read.sha256().to_string());
        let restored =
            Attachment::restore("shot.png", read.kind(), read.size(), read.sha256()).unwrap();
        assert_eq!(restored, read);
        assert_eq!(
            Attachment::restore("..", read.kind(), read.size(), read.sha256()),
            Err(NameError::Reserved)
        );
        assert_eq!(Dimensions::new(0, 1), None);
        assert_eq!(
            serde_json::from_value::<Attachment>(json.clone()).unwrap(),
            read
        );
        let mut zero = json.clone();
        zero["kind"]["dimensions"]["width"] = 0.into();
        assert!(serde_json::from_value::<Attachment>(zero).is_err());
        let mut path = json;
        path["name"] = "..".into();
        assert!(serde_json::from_value::<Attachment>(path).is_err());
    }

    #[test]
    fn base64_is_four_characters_for_every_three_bytes_or_part() {
        let encoded = |n| ByteSize::new(n).base64().get();
        assert_eq!([0, 1, 2, 3, 4].map(encoded), [0, 4, 4, 4, 8]);
    }

    #[test]
    fn each_client_declares_how_it_takes_images() {
        let image = Attachment::parse("a.png", &png(1, 1), &Limits::default())
            .unwrap()
            .kind();
        let takes = |harness: Harness| harness.acceptance().of(image);
        assert_eq!(takes(Harness::Codex), Takes::Always(Delivery::LocalPath));
        assert_eq!(
            takes(Harness::ClaudeCode),
            Takes::Always(Delivery::InlineBase64)
        );
        assert_eq!(takes(Harness::Pi), Takes::Always(Delivery::InlineBase64));
        let opencode = takes(Harness::OpenCode);
        assert_eq!(opencode.resolve(Offer::Offered), Ok(Delivery::InlineBase64));
        assert_eq!(
            opencode.resolve(Offer::NotOffered),
            Err(NotAccepted::NotOffered)
        );
        // Unknown is not yes.
        assert_eq!(opencode.resolve(Offer::Unknown), Err(NotAccepted::Unknown));
        assert_eq!(
            Takes::Never.resolve(Offer::Offered),
            Err(NotAccepted::Never)
        );
        assert_eq!(
            Takes::Always(Delivery::LocalPath).resolve(Offer::Unknown),
            Ok(Delivery::LocalPath)
        );
    }

    proptest! {
        #[test]
        fn any_bytes_are_read_or_refused_never_beyond_the_limits(
            bytes in proptest::collection::vec(any::<u8>(), 0..512),
            magic in 0..5usize,
        ) {
            let prefix: &[u8] = match magic {
                0 => b"\x89PNG\r\n\x1a\n",
                1 => &[0xFF, 0xD8, 0xFF],
                2 => b"GIF89a",
                3 => b"RIFF\0\0\0\0WEBP",
                _ => b"",
            };
            let input = [prefix, &bytes].concat();
            let limits = Limits::default();
            if let Ok(read) = Attachment::parse("x", &input, &limits) {
                let side = image(&read).dimensions().longest_side();
                prop_assert!(side > 0 && side <= 8000);
                prop_assert!(read.size() <= ByteSize::mebibytes(5));
            }
        }
    }
}
