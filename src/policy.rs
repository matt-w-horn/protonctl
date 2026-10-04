//! The write policy (RFC section 4, "Safety model"; Phase 1b). Each write
//! tool belongs to one class in `WRITE_TOOLS`, and `default_rule` decides
//! what the server does with that class once `writes = true` is set for the
//! tool's service (R11). Reads are not in the table: every read is offered.
#![expect(
    dead_code,
    reason = "the write tools that consult this arrive in Phase 1b"
)]

/// A class of write, at the grain the policy decides.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    /// Creates or edits a draft. It stays in the account: protonctl cannot send.
    Draft,
    /// Changes labels, read state or folder on mail; moves items within Drive.
    Organize,
    /// Creates Drive folders and uploads files.
    Create,
    /// Restores items from the trash.
    Restore,
    /// Moves items to the trash. Nothing is deleted permanently (R1).
    Trash,
}

/// What the server does with the tools of one class.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rule {
    /// Listed; the host's own approval settings decide each call.
    Allow,
    /// Listed with `_meta["anthropic/requiresUserInteraction"]`, so Claude
    /// Code asks a person for every call even where a rule would allow it.
    /// Desktop and Cowork go by the tool's annotations either way.
    Prompt,
    /// Not listed, and refused if called anyway.
    Deny,
}

/// Every write tool and its class (RFC section 4's tool table).
pub const WRITE_TOOLS: &[(&str, Op)] = &[
    ("create_draft", Op::Draft),
    ("update_draft", Op::Draft),
    ("modify_messages", Op::Organize),
    ("trash_messages", Op::Trash),
    ("create_folder", Op::Create),
    ("upload_file", Op::Create),
    ("move_files", Op::Organize),
    ("restore_files", Op::Restore),
    ("trash_files", Op::Trash),
];

/// The maintainer's rule: friction against risk, for each class of write.
pub fn default_rule(_op: Op) -> Rule {
    // A placeholder, which offers no write tool at all, until the maintainer
    // sets the rule in Phase 1b.
    Rule::Deny
}

#[cfg(test)]
mod tests {
    use super::*;

    /// R5: a person approves every trash call.
    #[test]
    fn trash_is_never_allowed_unasked() {
        assert_ne!(default_rule(Op::Trash), Rule::Allow);
    }
}
