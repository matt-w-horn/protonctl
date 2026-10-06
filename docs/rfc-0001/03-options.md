[RFC-0001](../rfc-0001.md) › 3. Options considered

# 3. Options considered

**Architecture**

| Option | Claude Code | Cowork | Crypto we write | Code we own | Verdict |
|---|---|---|---|---|---|
| Skill + official CLIs via Bash | yes | no (Linux VM shell) | none | prose | Claude Code stopgap only |
| **Thin host MCP adapter over Proton's clients** | yes | yes | none of Proton's; keyed pseudonyms from Phase 2 | ~3k lines planned; about 7.3k of code and 4.4k of tests built by 2026-10-04, plus the privacy layer | **chosen** |
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
| Calendar | direct-API read spike; Full-view ICS link; manual ICS files; browser; wait | Full-view ICS link (decided 2026-10-02) |
| Contacts | as Calendar, minus the link | out of scope (decided 2026-10-02) |

**Privacy layer (decided 2026-10-04)**

| Question | Options | Choice |
|---|---|---|
| Where privacy levels live | one config switch; tokenized tools plus raw single-item tools; tokenized only | both: one setting, `off` or `aliases` (Q26); in aliases mode, tokenized tools plus `reveal_*` tools behind user presence |
| How fine the setting is | per service; per host; per part (aliases, handles, keyed digests apart); one mode | one mode: a name shown plainly in one result and as an alias in another pairs them, and the parts protect only together ([section 4, Settings](04-design.md#settings)) |
| Mode of a new install | off; aliases; none, so setup asks | none, so setup asks: a wrong `off` cannot be recalled, a wrong `aliases` costs one command (Q27) |
| Gate for raw content | host permission prompt; MCP elicitation; Touch ID | Touch ID: users approve about 93% of permission prompts, hosts offer "Always allow", and Cowork does not render elicitation |
| Showing real names | a `decode` command; an MCP Apps panel; aliases kept in a Proton Pass vault; a `reveal_entity` tool | none: Claude pairs a name with its alias when a result shows both |
| Pseudonym form | per-process counters; short HMAC tokens; AES-SIV tokens in the prose; an alias plus an encrypted `ref` | three-word alias in the prose, `ref` in the `entities` table |
| Which entities | private individuals only (Privacy Filter's design); plus public figures; organizations in plaintext; an allowlist | all of them |
| Searches by name | no limit; a budget; Touch ID for new names | no limit; `guidance` steers toward topics and references |
| Images and scans | Vision OCR; a local vision-language model; text layer only | Vision OCR (Phase 4), a model later |
| Isolating converters | in process; a sandboxed helper; a helper in a Linux VM | sandboxed helper (Phase 4), VM later (Phase 7) |
| Detectors | regex; a name dictionary; GLiNER; Otter; Privacy Filter | regex and a dictionary (Phase 2), Otter (Phase 5, Q23); Privacy Filter dropped (Q39) |
| Digests | withhold; keyed; raw | keyed on both ends, as five words |
| Files on disk | export folder; crypto-shredded cache; RAM disk; none | in aliases mode none but a RAM disk for cloud-only Drive files, since `proton-drive` cannot stream (Q14); in off mode the download and export folders as built (Q26) |
| Local summaries | keep; drop; questions only | keep (Phase 6) |
| Calendar | keep, tokenized; off by default; remove | keep, tokenized; the link's risk is accepted |

Rejected outright: format-preserving encryption (an LLM needs no preserved
format, and NIST's 2025 draft of SP 800-38G Rev. 1 removes FF3); token
vaults; realistic fake names; differential-privacy rewriting; homomorphic
encryption and multiparty computation, which are far too slow.

**Language and MCP layer.** Rust (memory-safe, single binary) over Go
(go-proton-api exists, but a Go toolchain would be a second one) and
TypeScript (runtime and npm tree). MCP via the official `rmcp` 3.5, chosen
over a hand-rolled JSON-RPC loop and smaller community crates; its fast major-version churn is contained
by `Cargo.lock` and `--locked`.

**State.** A session-scoped stdio server, no daemon. Secrets in the Keychain,
not MCPB `sensitive` config (Desktop-only) or env vars in MCP config
(plaintext). No disk caches; in-memory caches per process (Drive index 60 s,
calendar feed 15 min); in aliases mode, from Phase 2, no content on disk
at all (R10).
Pseudonyms derive from one key, so they need no state (R14 to R17).
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
(`cargo llvm-cov --fail-under-lines`, 62% on 2026-10-03); pre-commit adds
gitleaks. Clippy reads the `[lints]` table in `Cargo.toml`: `pedantic`, a
set of off-by-default restriction lints and `print_stdout = "deny"`, since
stdout carries the MCP protocol. An override is an `#[expect]` with a reason
at its site. Unsafe code is forbidden in this crate (R18 weighs one
exception for LocalAuthentication, Q15). From Phase 2 the gate
also runs the leak test in aliases mode ([section 7](07-testing.md)): every tool, run against
synthetic data that holds planted identifiers, must return none of them.
The surface snapshot then covers both modes.

**Libraries.** A crate replaces hand-written code where one does the same
job: `tempfile` for download folders, `hex` for the pinned fingerprint, and
`regex` for the Drive CLI's node-UID pattern, copied from its source.
Evaluated on 2026-10-03 and declined: `icalendar`, whose parser makes one
pass over the whole feed, so a malformed invitation fails the calendar, and
which has no recurrence; `calcard`, which is lenient but expands recurrences
by counting from DTSTART with no window, so a daily series that began years
ago never reaches today, and protonctl would keep its own expansion and
convert `calcard`'s model back; `url`, since curl parses the link and a
second parser adds a differential, where the strict character check is the
control; and `globset`, `walkdir`, `shlex` and `toml_edit`, each of which
would save little or change behavior. Planned for the privacy layer:
`aes-siv` (RustCrypto; its README says it has never been audited), the
only primitive the tree lacks. HMAC-SHA-256, HKDF-SHA-256 and SHA-256 are
in `ring`, already a direct dependency for file digests (`src/digest.rs`),
but `ring` does not wipe its keys on drop, which R20 asks for; RustCrypto's
`hmac`, `hkdf` and `sha2` with their `zeroize` support do, at the cost of a
second implementation of SHA-256. Milestone M2.2 chose `ring`
([section 9](09-rollout.md)). `zeroize` (through `secrecy`) and `aho-corasick` (through `regex`),
for the name dictionary, are already in `Cargo.lock`. New: `phonenumber`
to validate phone numbers, a Unicode case-folding crate (the standard
library lowercases but does not case fold), and from Phase 5 `tract-onnx`
and `tokenizers` without its HTTP features (Q23). Each new crate's
licence is checked against `deny.toml`'s list before it is added.

---

[← 2. Requirements](02-requirements.md) · [Contents](../rfc-0001.md#contents) · [4. Design →](04-design.md)
