//! Errors in aliases mode carry a fixed code and text, never content (RFC
//! R13): no path, name, ID or text from Bridge, the CLI or the feed.

use serde_json::{Value, json};
use strum::IntoStaticStr;

use super::Mode;
use super::detect::Detector;

/// A service that could not be reached.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Service {
    Mail,
    Drive,
    Calendar,
}

/// A tool parameter, as a fault names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, IntoStaticStr)]
#[strum(serialize_all = "camelCase")]
pub enum Param {
    MessageId,
    ThreadId,
    EventId,
    FileId,
    FolderId,
    CalendarId,
    Path,
    Query,
    Index,
    Label,
    /// For a tool with no parameter that names an item.
    Item,
}

impl Param {
    pub fn name(self) -> &'static str {
        self.into()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fault {
    NotFound(Param),
    /// The broken rule, in fixed text that quotes no input.
    InvalidArgument(&'static str),
    InvalidHandle(Param),
    InvalidRef,
    InvalidPageToken,
    Unavailable(Service),
    Timeout,
    TooLarge,
    PrivacyModeUnset,
    PrivacyModeUnreadable,
    PrivacyModeChanged,
    PrivacyKeyMissing,
    PrivacyKeyUnreadable,
    PipelineFailed,
    Internal,
}

impl std::fmt::Display for Fault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message())
    }
}

/// So `?` can carry a fault through an operation's `anyhow` result.
impl std::error::Error for Fault {}

impl Fault {
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotFound(_) => "not_found",
            Self::InvalidArgument(_) => "invalid_argument",
            Self::InvalidHandle(_) => "invalid_handle",
            Self::InvalidRef => "invalid_ref",
            Self::InvalidPageToken => "invalid_page_token",
            Self::Unavailable(_) => "unavailable",
            Self::Timeout => "timeout",
            Self::TooLarge => "too_large",
            Self::PrivacyModeUnset => "privacy_mode_unset",
            Self::PrivacyModeUnreadable => "privacy_mode_unreadable",
            Self::PrivacyModeChanged => "privacy_mode_changed",
            Self::PrivacyKeyMissing => "privacy_key_missing",
            Self::PrivacyKeyUnreadable => "privacy_key_unreadable",
            Self::PipelineFailed => "pipeline_failed",
            Self::Internal => "internal",
        }
    }

    pub fn message(&self) -> String {
        match self {
            Self::NotFound(param) => format!("No item matches this {}.", param.name()),
            Self::InvalidArgument(rule) => (*rule).to_string(),
            Self::InvalidHandle(param) => format!(
                "This {} is not a handle from these tools, or the privacy key changed. Pass the value exactly as a result gave it.",
                param.name()
            ),
            Self::InvalidRef => "Use a ref from a result's entities table.".into(),
            Self::InvalidPageToken => "Pass nextPageToken exactly as returned.".into(),
            Self::Unavailable(Service::Mail) => {
                "Proton Mail Bridge cannot be reached; the user can run `protonctl doctor`.".into()
            }
            Self::Unavailable(Service::Drive) => {
                "Proton Drive cannot be reached; the user can run `protonctl doctor`.".into()
            }
            Self::Unavailable(Service::Calendar) => {
                "The calendar feed cannot be reached; the user can run `protonctl doctor`.".into()
            }
            Self::Timeout => "The call took longer than 150 s.".into(),
            Self::TooLarge => "Pass a smaller maxChars or pageSize.".into(),
            Self::PrivacyModeUnset => "No privacy mode is set; the user can run `protonctl setup privacy`, or `protonctl setup privacy --off`.".into(),
            Self::PrivacyModeUnreadable => {
                "The privacy setting cannot be read; the user can run `protonctl doctor`.".into()
            }
            Self::PrivacyModeChanged => {
                "The privacy setting changed; restart Claude Code or Claude Desktop.".into()
            }
            Self::PrivacyKeyMissing => {
                "The privacy key is missing; the user can run `protonctl setup privacy`.".into()
            }
            Self::PrivacyKeyUnreadable => {
                "The privacy key cannot be read; the user can run `protonctl doctor`.".into()
            }
            Self::PipelineFailed => {
                "protonctl could not tokenize this result, so it returns nothing.".into()
            }
            Self::Internal => {
                "The call failed inside protonctl; `protonctl doctor` shows more.".into()
            }
        }
    }

    /// The error result's JSON. The mode decides whether `detectors` is shown.
    pub fn json(&self, mode: Option<Mode>, detectors: &[Detector]) -> Value {
        let mut v = json!({ "error": self.code(), "message": self.message() });
        if mode == Some(Mode::Aliases) {
            v["detectors"] = json!(detectors);
        }
        v
    }
}
