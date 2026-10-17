# Security assessment

| | |
|---|---|
| **Scope** | protonctl on macOS: `src/`, `tests/`, `scripts/install.sh`, `scripts/serve`, `deny.toml`, `Cargo.toml`; and the design in [RFC-0002](rfc-0002.md) |
| **Threat model** | [RFC-0001, section 5](rfc-0001/05-security.md), for what is built; [RFC-0002, Stage 1 threat model](rfc-0002.md#stage-1-threat-model), for the proposal |
| **Fixes** | in the same tree as this file; each finding names the tests that hold its fix |
| **Earlier review** | the Phase 2 `/security-review` (Q25), whose record was deleted with aliases mode |

## Method

Two reviewers read the code whole. Claude (Opus 5.5) ran the assessment.
Gemini 4 Argon, through the Antigravity CLI, reviewed a copy of the
repository independently, in a read-only mode with no network, against
the attacker model below, and then reviewed RFC-0002's design. A finding
counts only after it is reproduced (a test, or a probe on the installed
binary) or refuted with evidence. Each fix comes with a test shown to
fail on the code before it.

Gemini's code review quoted RFC text that the RFC does not contain, in
two places, and described the Drive path fold as NFKC where the code uses
NFC. The code facts behind those items were checked separately; the
quotes and the NFKC claim were not used.

Attacker model, as in RFC-0001: a third party whose text and files reach
the Mac through mail, shared Drive files or invitations, and so reach the
model and the parsers; the model, acting on that text, calling any tool
with any arguments, and in Claude Code running commands as the user;
another program running as the user. Root and access to the Proton
account are out of scope.

## Findings

Severity is after verification, for this project's attacker model.

| ID | Finding | Severity | Status | Evidence |
|---|---|---|---|---|
| S1 | `Export::dir` made folders before it checked for symlinks, so a symlink planted in the export folder made protonctl create a folder inside the Proton Drive app's folder, which uploads it (R1, R10) | high | fixed | the export folder is opened one name at a time with no symlink followed, and each part is made and opened relative to the one above; `the_folder_must_be_absolute_and_outside_drive_and_the_cache`, `a_symlink_anywhere_on_the_way_is_refused_with_nothing_made_outside` |
| S2 | Downloads for the export folder were staged inside it; later manifest pages and saved attachments opened paths that a symlink or a hard link could redirect (R10) | high | fixed | downloads are staged in the private folder and copied out; files are created and opened relative to the folder held open, with `O_NOFOLLOW` and a single-link check; `an_export_is_staged_where_only_protonctl_can_write`, `a_manifest_page_refuses_a_link_swapped_in_for_its_file`, `an_exported_attachment_is_saved_once` |
| S3 | An invitation dated at the edge of chrono's range panics the calendar | none | refuted | such events are left out at parse time and the rest of the feed shows; `an_invitation_dated_at_chronos_limits_is_refused_not_a_panic` |
| S4 | The installed binary lacked the hardened runtime, so `DYLD_INSERT_LIBRARIES` loaded any library into it, and that code could read the Keychain items as protonctl with no prompt. This undid Q12's signing identity | high | fixed | a test library printed its marker inside the installed `protonctl`, and not inside a copy signed with `--options runtime`; `install.sh` now signs with it and refuses an install without it. It reaches the Mac at the next `install.sh` |
| S5 | Work that could outlast the 150 s limit or exhaust memory: an HTML quote scan quadratic in `<` characters, calendar expansion capped per event and not per query, whole threads fetched at once, attachments read whatever their size, and a `--version` run with no limit under the CLI's cross-process lock (R8) | medium | fixed | bounds in R8; `many_unclosed_tags_are_read_about_once`, `a_query_stops_at_the_cap_across_the_feed_and_says_so`, `expansion_stops_when_the_budget_runs_out`, `a_thread_is_fetched_a_bounded_size_at_a_time`, `an_attachment_over_64_mib_is_saved_not_read`, `a_version_check_that_hangs_is_killed_and_frees_the_lock`. Calendar expansion still runs on the async thread, within its 10 s budget |
| S6 | Runs of variation selectors and stray Hangul fillers passed R6, and can carry hidden text; the calendar's name and mailbox names were not cleaned | medium | fixed | R6 now keeps one selector after a visible base and fillers only inside jamo sequences; `selectors_and_fillers_stay_only_where_text_is_written_with_them`, `calendar_names_are_cleaned`, `hidden_characters_in_label_names_show_as_escapes`. One selector per character still passes (residual in section 5) |
| S7 | Exclusions: a symlink to a folder only in the cloud was listed by the symlink's own path; siblings whose names differ only in case were given each other's node IDs (R7) | medium | fixed | the CLI cannot follow a symlink, so the first was a wrong-folder bug and not a leak; both fixed; `a_symlink_to_a_folder_only_in_the_cloud_lists_where_it_leads`, `names_differing_only_in_case_keep_their_own_nodes`. An undecryptable ancestor of an excluded folder is not itself excluded, so showing it is not a leak; no change |
| S8 | The Drive CLI's private copy was checked, then ran `--version`, then ran the command, so the copy could change in the seconds between (R9) | medium | fixed | the command runs a fresh copy checked as the last step; the signature must be a Developer ID Application certificate of Proton's team; `the_command_runs_a_copy_checked_just_before_it`, `only_proton_s_developer_id_signature_passes` (by hand, on the real CLI) |
| S9 | curl read the calendar link without `--globoff`, so `[]` and `{}` in a link would make several requests (R3) | low | fixed | the link check refuses them and curl gets `--globoff`; `only_https_proton_links_pass` |
| S10 | A form feed inside a PDF page's text shifted the page count; the macOS readers have no memory limit; a reader's output is a channel back to the model | medium | partly fixed | form feeds inside a page become line breaks; `form_feeds_in_a_page_s_text_do_not_split_it`. The memory limit stays open (RFC-0001, section 9). The output channel is the readers' purpose and is accepted |
| S11 | In `serve`, a library logged a refused request's method name at WARN, a startup error printed the config's path, and the sweep printed a folder's path (R25) | low | fixed | the libraries' log output is off in `serve`, and errors there print fixed lines; `refused_requests_stay_out_of_stderr`, `a_server_that_cannot_start_names_no_path_or_address` |
| S12 | `doctor`'s reader check had no time limit and did not kill a stuck reader | low | fixed | `a_sample_run_that_never_ends_stops_at_its_limit` |
| S13 | Non-ASCII search terms go to Bridge as 8-bit quoted strings, not literals | info | accepted | Bridge accepts them, and line breaks and NULs are refused before quoting (`quoted` in `src/mail/query.rs`) |
| S14 | Three RustSec advisories (RUSTSEC-2026-0370 to 0372) mark `zvariant`, `zbus_names` and `zvariant_derive` unmaintained, failing `cargo deny` and so every commit | low | accepted | Linux only, through `secret-service` 5.2, the newest; the crates were merged into zbus 6, not abandoned; ignored with a reason in `deny.toml` until `secret-service` moves |
| S15 | A Proton Doc or Sheet read as empty text, and `download_file` failed with the CLI's raw report. Proton documents that moving or copying a Doc's placeholder out of the Drive folder sends the Doc to Trash | low | fixed | protonctl never moves or copies a file out of the Drive folder; each Drive tool now names the export from the web app; `proton_docs_and_sheets_say_how_to_read_them` |

## What holds

- R1: Bridge mailboxes are opened with EXAMINE and bodies fetched with
  `BODY.PEEK`, and the scripted Bridge refuses every other command; the
  stand-in Drive CLI answers only the four calls protonctl makes. With
  S1 fixed, no tool makes anything inside the Drive app's folder.
- R2: secrets come from the Keychain only, reach curl on stdin, and are
  never logged; with S4 fixed, no other program reads them as protonctl.
- R3: protonctl's own sockets go to 127.0.0.1; the one internet request
  goes through `/usr/bin/curl`, to `https` on `proton.me`, now with
  `--globoff`.
- R4, R6, R7, R8, R9, R10, R12, R21 and R25, as RFC-0001 states them,
  with the changes above.

## Residual risks

The residual risks are in [RFC-0001, section 5](rfc-0001/05-security.md#residual-risks).
This assessment added four: the macOS readers' missing memory limit, one
variation selector per character, calendar expansion on the async thread
within its budget, and the terminal check that a pseudo-terminal passes.

## The RFC-0002 design review

Gemini's design review of an earlier draft of RFC-0002, which proposed a
Drive workspace, mail organizing and staged edits all at once, found
these, each checked against the draft and Proton's documented behavior:

- The sharing check covered the workspace and the folders above it, but
  not the target or the folders below the workspace, so a subfolder the
  user shared would receive what Claude wrote.
- Copying into the workspace read its source without applying exclusions
  (R7).
- A pseudo-terminal passes a "terminal only" check, and the signed
  protonctl driven through one meets no Keychain prompt, so no
  terminal-gated policy is a boundary against the model.
- Replacing or discarding a draft deletes it permanently in Bridge, even
  after the user edited it.
- A recipient rule limited to the thread does not stop a reply to an
  attacker who wrote the thread, and HTML drafts can hide text in a
  collapsed quote or load a remote image.
- Removing a label in Bridge takes `UID EXPUNGE`, and archiving out of a
  custom folder loses where the message was.
- A write log and a proposal queue the model can write are not records
  to undo from.
- Server-side copies use storage quota that Proton Mail shares, and a
  full quota stops incoming mail.

The draft was replaced: RFC-0002 now proposes Stage 1 only (new files in
the local export folder, plain-text drafts that are never replaced or
removed, labels added under a prefix), and records these findings as the
conditions for Stages 2 and 3.

## Not covered

- Linux. Its readers' sandbox, the Secret Service store and the memfd copy
  of the Drive CLI were reviewed earlier and are not reviewed here; the
  fixes above were built and tested on macOS only.
- Proton's clients and Anthropic's hosts, which are out of scope
  ([SECURITY.md](../SECURITY.md)).
- A live prompt-injection drill (T12).
