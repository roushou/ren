//! Where each session's attachments are kept: one folder per operator
//! session under the attachments folder Ren owns, one folder per attachment
//! in it, holding the file under its name. An upload is written first under
//! the folder's `.incoming`, then moved into place whole. Pure: the host
//! makes, reads, and removes the files.

use ren_harness::attachment::{Attachment, AttachmentName};
use ren_recorder::id::{AttachmentId, SessionId};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// The attachments folder: `<root>/<session>/<attachment>/<name>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AttachmentFolders {
    root: PathBuf,
}

impl AttachmentFolders {
    /// Where uploads are written before they are moved into place.
    const INCOMING: &'static str = ".incoming";

    pub(crate) fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// The folder holding every attachment of `session`.
    pub(crate) fn session(&self, session: SessionId) -> PathBuf {
        self.root.join(session.to_string())
    }

    /// The file `attachment` of `session` is, named `name`.
    pub(crate) fn file(
        &self,
        session: SessionId,
        attachment: AttachmentId,
        name: &AttachmentName,
    ) -> PathBuf {
        self.session(session)
            .join(attachment.to_string())
            .join(name.as_str())
    }

    /// The folder holding `attachment` alone.
    pub(crate) fn attachment(&self, session: SessionId, attachment: AttachmentId) -> PathBuf {
        self.session(session).join(attachment.to_string())
    }

    /// Where `attachment` is written before it is moved into place.
    pub(crate) fn incoming(&self, attachment: AttachmentId) -> PathBuf {
        self.root.join(Self::INCOMING).join(attachment.to_string())
    }
}

/// A session's attachments, by id.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Attached(BTreeMap<AttachmentId, Attachment>);

impl Attached {
    pub(crate) fn from_stored(stored: Vec<(AttachmentId, Attachment)>) -> Self {
        Self(stored.into_iter().collect())
    }

    pub(crate) fn get(&self, attachment: AttachmentId) -> Option<&Attachment> {
        self.0.get(&attachment)
    }

    pub(crate) fn insert(&mut self, attachment: AttachmentId, read: Attachment) {
        self.0.insert(attachment, read);
    }

    pub(crate) fn remove(&mut self, attachment: AttachmentId) -> Option<Attachment> {
        self.0.remove(&attachment)
    }
}

#[cfg(test)]
mod tests {
    use super::AttachmentFolders;
    use ren_harness::attachment::AttachmentName;
    use ren_recorder::id::{AttachmentId, SessionId};
    use std::path::Path;
    use uuid::Uuid;

    #[test]
    fn each_session_and_attachment_has_its_own_folder() {
        let folders = AttachmentFolders::new("/data/attachments".into());
        let session = SessionId::restore(Uuid::from_u128(1));
        let attachment = AttachmentId::restore(Uuid::from_u128(2));
        let name = AttachmentName::parse("../shot.png").unwrap();
        assert_eq!(
            folders.file(session, attachment, &name),
            Path::new(
                "/data/attachments/00000000-0000-0000-0000-000000000001/00000000-0000-0000-0000-000000000002/shot.png"
            )
        );
        assert!(
            folders
                .file(session, attachment, &name)
                .starts_with(folders.attachment(session, attachment))
        );
        assert_eq!(
            folders.incoming(attachment),
            Path::new("/data/attachments/.incoming/00000000-0000-0000-0000-000000000002")
        );
    }
}
