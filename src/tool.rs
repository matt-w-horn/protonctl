//! The MCP tools by name. A tool's name is text only in its `#[tool]`
//! registration and on the wire; everything else names it by this type,
//! and a test in `serve.rs` holds the two to the same set.

use strum::{EnumString, IntoStaticStr, VariantArray};

use crate::privacy::error::{Param, Service};

#[derive(Clone, Copy, Debug, PartialEq, Eq, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
pub enum Tool {
    GetStatus,
    ListCalendars,
    ListEvents,
    SearchEvents,
    GetEvent,
    SearchFiles,
    ListFolder,
    GetFileMetadata,
    ReadFileContent,
    DownloadFile,
    ListDriveTree,
    ExportDriveManifest,
    SearchThreads,
    CountMessages,
    GetMessage,
    GetThread,
    ListLabels,
    GetAttachment,
}

impl Tool {
    /// The tool's name on the wire.
    pub fn name(self) -> &'static str {
        self.into()
    }

    /// The service an untyped failure is blamed on in aliases mode, where
    /// the operation's own error text never leaves (R13).
    pub fn service(self) -> Option<Service> {
        match self {
            Self::GetStatus => None,
            Self::ListCalendars | Self::ListEvents | Self::SearchEvents | Self::GetEvent => {
                Some(Service::Calendar)
            }
            Self::SearchFiles
            | Self::ListFolder
            | Self::GetFileMetadata
            | Self::ReadFileContent
            | Self::DownloadFile
            | Self::ListDriveTree
            | Self::ExportDriveManifest => Some(Service::Drive),
            Self::SearchThreads
            | Self::CountMessages
            | Self::GetMessage
            | Self::GetThread
            | Self::ListLabels
            | Self::GetAttachment => Some(Service::Mail),
        }
    }

    /// The parameter a "not found" is about in aliases mode, the only mode
    /// whose errors are codes, when the operation's error does not say.
    pub fn missing(self) -> Param {
        match self {
            Self::GetStatus => Param::Item,
            Self::ListCalendars | Self::ListEvents | Self::SearchEvents => Param::CalendarId,
            Self::GetEvent => Param::EventId,
            Self::SearchFiles | Self::ListFolder | Self::ListDriveTree => Param::FolderId,
            Self::GetFileMetadata | Self::ReadFileContent => Param::FileId,
            Self::DownloadFile | Self::ExportDriveManifest => Param::Path,
            Self::SearchThreads | Self::CountMessages => Param::Query,
            Self::GetMessage | Self::GetAttachment => Param::MessageId,
            Self::GetThread => Param::ThreadId,
            Self::ListLabels => Param::Label,
        }
    }
}
