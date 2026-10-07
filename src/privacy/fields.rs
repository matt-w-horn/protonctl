//! Field policies (RFC section 6; API specification "MCP tools"): what each
//! result field is, by its name and the tool. The same name means the same
//! thing across tools, so the table is by name, with the few differences by
//! tool. A string field the table does not name is removed and listed in
//! `dropped`, never passed on: the detectors cannot see an opaque ID.

use super::ident::{Algorithm, ItemKind, TokenKind};
use crate::tool::Tool;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Policy {
    /// Written by protonctl, or a date, count, size or enum: kept.
    Plain,
    /// Text people wrote: detectors and the dictionary run on it.
    Text,
    /// "Name <email>" or "email".
    Address,
    /// One address or one domain, the whole value: a group's key.
    AddressOrDomain,
    /// A MIME type, which a sender can write: a known one stays.
    MimeType,
    /// `{name, email, response}`.
    Person,
    /// A Drive path: each part as Text; its handle goes beside it.
    DrivePath,
    /// A path on this machine: a fixed placeholder.
    LocalPath,
    Id(ItemKind),
    /// A digest, in hex, keyed under the digest key; the algorithm's name.
    Digest(Algorithm),
    Token(TokenKind),
    /// Removed, with what it holds (R16, R10).
    Drop,
    /// An error's text inside a successful result: a fault code only.
    Error,
    /// An image's description: images are not returned in aliases mode.
    Image,
    /// An object or array: its members get their own policies.
    Within,
    /// No rule: a string is removed and listed in `dropped`.
    Unlisted,
}

/// The policy for a member named `key` in `tool`'s result; for an array's
/// items, the array's own name.
#[expect(
    clippy::match_same_arms,
    reason = "grouped by service, as the API specification lists them"
)]
pub fn policy(tool: Tool, key: &str) -> Policy {
    use Policy::{
        Address, AddressOrDomain, Digest, DrivePath, Drop, Error, Id, Image, LocalPath, MimeType,
        Person, Plain, Text, Token, Within,
    };
    match key {
        // Mail
        "messageId" => Id(ItemKind::Message),
        "threadId" => Id(ItemKind::Thread),
        "from" | "to" | "cc" => Address,
        "subject" | "snippet" | "body" | "authentication" | "searched" => Text,
        "mimeType" => MimeType,
        "key" => AddressOrDomain,
        // Calendar
        "eventId" => Id(ItemKind::Event),
        "summary" | "description" | "location" => Text,
        "organizer" | "attendees" => Person,
        // Drive
        "path" | "parent" => DrivePath,
        "nodeId" | "revisionId" | "uid" | "parentUid" => Drop,
        "sha256" => Digest(Algorithm::Sha256),
        "sha1" | "claimedSha1" => Digest(Algorithm::Sha1),
        "protonError" | "error" => Error,
        "image" => Image,
        // Pages of images and saved files are not part of aliases mode (R10, R22).
        "pdfPages" | "pageStarts" | "pagesShown" | "nextPage" | "textLayer" | "savedTo"
        | "inline" | "downloads" | "export" => Drop,
        // get_status
        "config" | "cli" => LocalPath,
        "folder" if tool == Tool::GetStatus => LocalPath,
        // The Drive CLI's pin on Linux: a digest the model has no use for.
        "cliSha256" => Drop,
        "address" | "secretsHeld" => Text,
        "nextPageToken" => Token(TokenKind::of(tool)),
        // Shared
        "name" | "content" | "hiddenNames" => Text,
        "date" | "start" | "end" | "status" | "showsAs" | "fetchedAt" | "access" | "freshness"
        | "window" | "note" | "kind" | "source" | "reason" | "textFrom" | "origin"
        | "trashAndSpam" | "by" | "use" | "encryption" | "provenance" | "version" | "cannot"
        | "revoke" | "calendarId" | "response" | "localModified" | "modified"
        | "claimedModified" | "mode" | "detectors" => Plain,
        "messages" | "message" | "calendars" | "events" | "event" | "file" | "files" | "items"
        | "rows" | "groups" | "newest" | "oldest" | "labels" | "attachments" | "unlisted"
        | "proton" | "mail" | "drive" | "privacy" => Within,
        _ => Policy::Unlisted,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_and_paths_never_pass_as_plain() {
        for key in [
            "messageId",
            "threadId",
            "eventId",
            "nodeId",
            "revisionId",
            "sha256",
            "path",
            "nextPageToken",
        ] {
            assert!(
                !matches!(
                    policy(Tool::GetStatus, key),
                    Policy::Plain | Policy::Text | Policy::Unlisted
                ),
                "{key}"
            );
        }
        assert_eq!(policy(Tool::GetStatus, "a_new_field"), Policy::Unlisted);
        assert_eq!(
            policy(Tool::GetThread, "nextPageToken"),
            Policy::Token(TokenKind::ThreadPage)
        );
    }
}
