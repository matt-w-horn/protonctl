[RFC-0001](../rfc-0001.md) › 3. Options considered

# 3. Options considered

**Architecture**

| Option | Claude Code | Cowork | Crypto we write | Code we own | Verdict |
|---|---|---|---|---|---|
| Skill + official CLIs via Bash | yes | no (Linux VM shell) | none | prose | Claude Code stopgap only |
| **Thin host MCP adapter over Proton's clients** | yes | yes | none of Proton's; the keyed pseudonyms of Phase 2 were withdrawn with aliases mode (Q43) | ~3k lines planned; about 7.3k of code and 4.4k of tests built first, plus the privacy layer, since withdrawn (Q43) | **chosen** |
| Native Rust client of Proton's API | yes | yes | SRP, key unlock, every format | 6k to 10k lines on top of the reads | rejected: it owns Proton's cryptography, and Proton refuses an honest third-party identity for Mail and Calendar (Q4); rejected for Calendar/Contacts too (Q1) |
| TypeScript server on the Drive SDK + Bridge | yes | yes | none for Drive | ~2k + Node + npm tree | a Node runtime to install; GPL crypto peer dependency |
| Adopt community servers | yes | yes | theirs | 3 codebases in 3 languages | review burden, bus factor of 1 |
| Claude in Chrome on Proton's web apps | yes | yes | none | 0 | occasional calendar edits only |
| Hosted connector like Google's | yes | yes | n/a | n/a | a server would hold Proton keys |

**Backend per service**

| Service | Options | Choice |
|---|---|---|
| Mail | Bridge IMAP; direct API (unsupported identity); Export Tool EML (stale); SMTP token (send-only, not E2EE) | Bridge IMAP, no SMTP |
| Drive | folder for namespace + CLI for the rest; CLI only (needs rate-limited tree walks); folder only (no trash semantics, dataless timeouts); SDK direct (no auth module) | folder for search/list/stat, CLI for the rest |
| Calendar | direct-API read spike; Full-view ICS link; manual ICS files; browser; wait | Full-view ICS link (decided) |
| Contacts | as Calendar, minus the link | out of scope (decided) |

**Privacy layer (decided).** Withdrawn later with aliases mode
([Q43](10-open-questions.md)). Its table of options is deleted. The
questions it answered are marked withdrawn in
[section 10](10-open-questions.md), and the options it rejected outright
are in that section's list of what was declined with the privacy layer.

**Language and MCP layer.** Rust (memory-safe, single binary) over Go
(go-proton-api exists, but a Go toolchain would be a second one) and
TypeScript (runtime and npm tree). MCP via the official `rmcp` 3.5, chosen
over a hand-rolled JSON-RPC loop and smaller community crates; its fast major-version churn is contained
by `Cargo.lock` and `--locked`.

**State.** A session-scoped stdio server, no daemon. Secrets in the Keychain,
not MCPB `sensitive` config (Desktop-only) or env vars in MCP config
(plaintext). No disk caches; in-memory caches per process (Drive index 60 s,
calendar feed 15 min); content on disk only in the download and export
folders (R10).
Calendar fetch through `/usr/bin/curl`, not an HTTP crate. Bridge is opened
on demand when nothing answers on its port (Q2).

**Packaging.** `scripts/install.sh` (build, sign, install; Q12), then one
`claude mcp add` and one `claude_desktop_config.json` entry. protonctl is
also a Claude Code plugin, because Anthropic's plugin directory accepts a
local MCP server only inside a plugin: a connector must be a remote
`https://` server, and MCP Bundles are no longer accepted. The plugin's
launcher, `scripts/serve`, starts the installed binary (Q36).
Each rebuild changes the ad-hoc signature, so the Keychain asks again for
every item, and approving that from habit would also approve a replaced
binary. Only a stable signing identity stops the prompt after a rebuild,
since an ad-hoc signature's requirement is the binary's own hash; an
install path the user cannot write stops a swap but not the prompt. From
Phase 2 protonctl is signed with a self-signed certificate (Q12).

**Gates.** `scripts/check.sh` runs `cargo fmt --check`, clippy with
`-D warnings`, the tests, `cargo deny check` (advisories, licences, sources,
HTTP-crate bans) and a line-coverage floor
(`cargo llvm-cov --fail-under-lines`, 62%); pre-commit adds
gitleaks. Clippy reads the `[lints]` table in `Cargo.toml`: `pedantic`, a
set of off-by-default restriction lints and `print_stdout = "deny"`, since
stdout carries the MCP protocol. An override is an `#[expect]` with a reason
at its site. Unsafe code is forbidden in this crate. On Linux the gate also runs
the Secret Service tests against a throwaway GNOME Keyring and, where
podman is installed, every mail read against Dovecot (T11).

**Libraries.** A crate replaces hand-written code where one does the same
job: `tempfile` for download folders, `hex` for the pinned fingerprint, and
`regex` for the Drive CLI's node-UID pattern, copied from its source.
Evaluated and declined: `icalendar`, whose parser makes one
pass over the whole feed, so a malformed invitation fails the calendar, and
which has no recurrence; `calcard`, which is lenient but expands recurrences
by counting from DTSTART with no window, so a daily series that began years
ago never reaches today, and protonctl would keep its own expansion and
convert `calcard`'s model back; `url`, since curl parses the link and a
second parser adds a differential, where the strict character check is the
control; and `globset`, `walkdir`, `shlex` and `toml_edit`, each of which
would save little or change behavior. Each new crate's
licence is checked against `deny.toml`'s list before it is added.

---

[← 2. Requirements](02-requirements.md) · [Contents](../rfc-0001.md#contents) · [4. Design →](04-design.md)
