[RFC-0001](../rfc-0001.md) › 10. Open questions

# 10. Open questions

| Q | Topic | State | Needed by |
|---|---|---|---|
| Q1 | Calendar through a Full-view link; Contacts out | decided 2026-10-02 |  |
| Q2 | Open Bridge on demand | decided 2026-10-03 |  |
| Q3 | No decoded body scan | decided 2026-10-03 |  |
| Q4 | Run without Bridge, the Drive app and the link | answered 2026-10-03 |  |
| Q5 | Read-only | decided 2026-10-04 |  |
| Q6 | Shape of the privacy layer | decided 2026-10-04 |  |
| Q7 | Every entity tokenized | decided 2026-10-04 |  |
| Q8 | Stable identifiers without state | decided 2026-10-04 |  |
| Q9 | Calendar stays, tokenized | decided 2026-10-04 |  |
| Q10 | No content on disk; sandboxed converters | decided 2026-10-04 |  |
| Q11 | All services tokenized at once | decided 2026-10-04 |  |
| Q26 | The privacy layer is optional | decided 2026-10-04; its proposal to confirm | M2.0 |
| Q12 | Signing identity and install path | open | Phase 2 |
| Q13 | `sandbox-exec` still enforced | open | Phase 4 |
| Q14 | Drive downloads to stdout, or a RAM disk | open | Phase 2 |
| Q15 | Reaching LocalAuthentication | open | Phase 3 |
| Q16 | Token cost of words and base64url | open | before Phase 2 |
| Q17 | Claude Code sandbox settings | open | Phase 3 |
| Q18 | Word list licence and curation | open | Phase 2 |
| Q19 | Alias input, word count, URLs, what a `ref` holds | open | Phase 2 |
| Q20 | Drive handles: path or node UID | open | Phase 2 |
| Q21 | Pairing names with aliases | open | Phase 2 |
| Q22 | Further entity types; dictionary scope | open | Phase 2 |
| Q23 | Model runtime for Phases 5 to 7 | open | Phase 5 |
| Q24 | Check the CLI's signature before every run | open | Phase 2 |
| Q25 | `/security-review` for Phases 2 and 3 | open | Phase 2 |
| Q27 | Mode of a new install | open | Phase 2 |
| Q28 | Where the mode lives | open | Phase 2 |
| Q29 | Linux is supported beside macOS | decided 2026-10-04 | |
| Q30 | How platform code is structured | open, B and C recommended | P1 |
| Q31 | The secret store on Linux | open | P2 |
| Q32 | Starting Bridge on Linux | open | P2 |
| Q33 | Drive on Linux: the CLI, its check, no app folder | open | P3 |
| Q34 | Converters and their sandbox on Linux | open | P4 |
| Q35 | User presence on Linux | open | P5 |

## Decisions and questions

- Q1, decided 2026-10-02: Full-view ICS link, calendar read-only; Contacts
  out. Revisit if Proton ships a calendar or contacts API.
- Q2, decided 2026-10-03: Bridge on demand. Its cold start is about 10 s to
  a working login, under the 20 s this RFC set, so when nothing answers on
  its port protonctl opens the Bridge app hidden
  (`open -g -j -b com.protonmail.bridge`) and retries for up to 40 s. It opens
  the app rather than the headless core, which would collide with the app
  the user opens themselves; a look-alike app gains nothing, since the
  certificate pin refuses it. Measured: with Bridge quit, a search took
  14.4 s and left Bridge running. `setup mail` never opens it.
- Q3, decided 2026-10-03: no decoded scan. Bridge's raw-byte body search finds
  plain words in 1.5 s across the whole mailbox; the search tool's description
  says it can miss accented words and words split at line wraps.
- Q4, answered 2026-10-03: run without Bridge, the Drive app and the
  calendar link? [Appendix B](appendix-b-proton-code.md) surveys Proton's code and lists the options.
  Decided: the client may handle the Proton password directly. But Proton's
  API refuses an honest third-party identity for Mail and Calendar
  at its first request (code 5002, tested without an account), so Bridge and
  the calendar link stay unless Proton grants one. Drive's third-party
  identity is accepted, as its SDK documents, and the Drive app is now
  optional (option 1).
- Q5, decided 2026-10-04: read-only. Phase 1b, R5 and R11 are withdrawn, and
  R1 now forbids any change to the account.
- Q6, decided 2026-10-04: the shape of the privacy layer. The existing tool
  names return tokenized results; raw content comes only from `reveal_*`
  tools, one handle per call, behind user presence (R18). Separate names,
  because Claude Code's permission rules match tool names, not arguments.
  Touch ID, because hosts offer "Always allow" and Cowork does not render
  elicitation. Since Q26 this is the shape of aliases mode; off mode has no
  `reveal_*` tools and returns results as built.
- Q7, decided 2026-10-04: every entity is tokenized, public ones included;
  searches by name are not limited, and `guidance` steers toward topics and
  references.
- Q8, decided 2026-10-04: identifiers stay stable for months without state:
  aliases and keyed digests as words, references and handles as AES-SIV
  ciphertexts. A long chat, an agent's notes or Claude's memory can keep
  them.
- Q9, decided 2026-10-04: Calendar stays, tokenized. The share link's risk,
  that Proton's server decrypts the calendar on each fetch, is accepted.
- Q10, decided 2026-10-04: no content on disk; converters move into a
  sandboxed helper; Vision OCR now and a local vision-language model later;
  a Linux VM helper for the riskiest formats later still. Since Q26, no
  content on disk holds in aliases mode, and off mode keeps the download
  and export folders (proposed with Q26); the sandboxed converters hold in
  both modes.
- Q11, decided 2026-10-04: all three services get tokenization at once,
  with regex and a name dictionary, and GLiNER after; services need not
  fail closed during the move. Local summaries stay, after GLiNER.
  Read on review (2026-10-04) as: no service is switched off while Phase 2
  is built, since all three ship together; once Phase 2 ships, a pipeline
  failure fails the call (R13). If a raw fallback was meant instead, R13's
  fail-closed rule is the place to change.
- Q26, decided 2026-10-04: the privacy layer is optional, since some users
  want a read-only stand-in for Claude's Google connectors without it.
  Proposed with it, to confirm in M2.0 ([section 4, Settings](04-design.md#settings)): one mode,
  `off` or `aliases`, for every service and host on the Mac; off mode keeps
  the 18 tools as built, with their parameters, page images, downloads and
  the export folder; a service is on only once set up, Drive through a new
  `setup drive`; a running server refuses calls after a mode change, and
  aliases mode refuses without its key (R26).
- Q29, decided 2026-10-04: Linux is supported beside macOS on Apple
  silicon, so that protonctl builds and tests in cloud containers and
  serves Linux users. Windows stays out. A feature with no Linux mechanism
  that meets its requirement is absent on Linux ([section 11](11-platforms.md)).
- Open, to check before the phase that depends on each:
  - Q12 (Phase 2): sign protonctl with a stable identity, so that a
    Keychain prompt after a rebuild stops being routine, and install it to
    a path the user cannot write, so that a swap needs an admin. The path
    alone does not stop the prompt: an ad-hoc signature's requirement is
    the binary's hash. Identities, cheapest first: a self-signed
    code-signing certificate in the login keychain (free; its private key
    is readable by the user's own processes after a prompt), an Apple
    Development certificate, or a Developer ID.
  - Q13 (Phase 4): `sandbox-exec` is marked deprecated in its man page.
    Check that it still enforces a profile on the current macOS, and that
    Vision, PDFKit through `osascript`, and `textutil` run under a profile
    that denies network access and file writes.
  - Q14 (Phase 2): can `proton-drive` write a download to stdout, or to a
    named pipe? If not, R10's RAM disk is needed; then check that it can be
    made and removed without an admin, that it honours ownership (macOS can
    mount disk images with ownership ignored), and that it can stay out of
    Finder and Spotlight.
  - Q15 (Phase 3): does a LocalAuthentication prompt appear when protonctl
    runs as a child of Claude Desktop, as a child of Claude Code, and inside
    Claude Code's sandbox? Which route reaches it with unsafe code
    forbidden: an `osascript` script, a Swift helper, or `objc2` with one
    reviewed exception (R18)? Whichever starts the prompt is the process
    macOS names in it.
  - Q16 (before Phase 2): measure with Anthropic's token-counting API
    whether words cost fewer tokens than base64url for aliases, references,
    handles and keyed digests.
  - Q17 (Phase 3): which Claude Code sandbox settings deny reading the
    Drive app's folder, running `proton-drive`, connecting to Bridge's
    port, writing protonctl's config, running `security` on protonctl's
    Keychain items, and reaching hosts outside an allowlist?
  - Q18 (Phase 2): the EFF word list's license, and the curation rule: keep
    at least 7,132 words, so that five words hold 64 bits, after dropping
    loaded words, names, brands, and words that name a kind of place,
    organization, role or relation ([Appendix C](appendix-c-roleplay.md)).
  - Q19 (Phase 2): the alias. Whether names (person, organization,
    location) enter the HMAC with their type, so that a detector's change
    of type changes the alias, or without it; how a change to the canonical
    rules or the word list is announced (a version in `detectors`, or a
    key rotation); three words, four (about 51 bits: a collision across a
    million entities about 0.02%), or three with a fourth only on a
    collision in one result; whether URLs get word aliases and `ref`s at
    all, or a per-result number with the host's alias; and whether a `ref`
    holds only the canonical value, or also the forms seen and the
    person's addresses, so that a search by `ref` finds "Chen, Alice".
  - Q20 (Phase 2): the Drive handle. The path as stored, padded, so that
    exclusions apply by path, but stale after any rename or move above the
    file; or the node UID, stable across renames, resolved to a path at
    each use (a CLI `info` per call where the app's folder cannot map it).
  - Q21 (Phase 2): pairing names with aliases. `queryEntities` on every
    query, only when the name matches, or never (Claude then cannot link
    the name it typed); `reveal_*` results with the `entities` pairing or
    without; key epochs, a scheduled `rotate-key`, which bound how far one
    pairing reaches but end Q8's stability at each epoch.
  - Q22 (Phase 2): further entity types: government ID numbers (SSN,
    passport, tax IDs), credentials and one-time codes in mail, and local
    account names in paths; and whether the dictionary covers the whole
    process (every correspondent's name from All Mail's ENVELOPE, held in
    memory) rather than one call.
  - Q23 (Phase 5): the model runtime for GLiNER, the Phase 6 local model
    and Privacy Filter: in process (`ort`, unsafe code in a dependency
    only), inside the sandboxed converter, or a local server on 127.0.0.1,
    which R3 would have to name and `deny.toml`'s HTTP-crate bans would
    have to allow; weights shipped with the install and pinned by SHA-256;
    their licences.
  - Q24 (Phase 2): R9's check runs once per process and the CLI then runs
    by a path the user can write. Check the signature before every run
    (one `codesign` call each), or require the CLI at a path the user
    cannot write?
  - Q25 (Phases 2 and 3): `/security-review` on the phases that add
    protonctl's own cryptography and user presence, beside `/code-review`?
    Declined on 2026-10-02, before either existed.
  - Q27 (Phase 2): the mode of a new install. (a) None: until a mode is
    set, every call answers with the two `setup privacy` commands, and
    `doctor` fails. Recommended, because the two errors cost different
    amounts: a wrong `off` sends names to the provider, where they cannot
    be recalled, and a wrong `aliases` costs one command to undo. (b)
    `off`: a stand-in with no extra step, as built, and an opt-in that
    comes with reading what Phase 2 does not cover (names in free text
    until Phase 5); Lockdown Mode and Advanced Data Protection are opt-in
    too. (c) `aliases`: protects whoever skips the docs, at the cost of
    friction for stand-in use while coverage is partial. Under (a), the
    maintainer's install chooses once when Phase 2 ships.
  - Q28 (Phase 2): where the mode lives. In the config it is visible and
    `doctor` can explain it, but `src/config.rs` only appends, so changing
    the mode needs an edit in place (`toml_edit`, declined on 2026-10-03
    when nothing was edited in place) or a hand edit; and in Claude Code
    the model can edit the file unless the sandbox denies it (Q17). In a
    Keychain item beside the key, no config edit is needed, and a change
    from outside protonctl meets a Keychain prompt (to confirm for
    deletion, which would otherwise read as a missing key and refuse calls
    rather than fall back). Also: whether `setup privacy --off` asks for
    user presence (R18's route, from Phase 3), so that injected text
    running the CLI cannot turn the layer off.
  - Q30 (P1): how platform code is structured. Recommended: one platform
    module per system chosen by `cfg`, behind a trait per service, with
    target-specific dependencies; features only for optional backends;
    cross-platform crates where no control is lost; a workspace split
    later. [Section 11](11-platforms.md) weighs the seven options.
  - Q31 (P2): the secret store on Linux: the Secret Service (recommended),
    kernel keyutils (lost at reboot), or `pass`; and with no Secret
    Service running, refuse rather than write a file.
  - Q32 (P2): Bridge on Linux: start its core with `--noninteractive`,
    leave it to a systemd user unit, or require it running (recommended
    first).
  - Q33 (P3): Drive on Linux: whether Proton ships the Drive CLI for
    Linux, how to check it without `codesign` (a SHA-256 pinned at
    `setup drive`, or Proton's signature), and the CLI-only mode, since no
    Linux Drive app with a local folder is known.
  - Q34 (P4): converters on Linux: poppler-utils, pandoc or LibreOffice,
    and Tesseract as external tools, or Rust crates inside the helper; the
    sandbox: Landlock and seccomp (recommended) or bubblewrap.
  - Q35 (P5): user presence on Linux: polkit with an authentication
    agent, fprintd, a FIDO2 key's touch, or no `reveal_*` on Linux.

- Declined on 2026-10-03:
  - per-label and per-folder scope levels: out of scope for this package;
  - downloading cloud-only files only to hash them: the SHA-1 Proton stores
    at upload serves joins with no download;
  - materializing Drive files in place: the export folder is the one
    outlet, in off mode only from Phase 2;
  - a log of every call;
  - batch search and `from_domain:` or `rcpt:` operators: `count_messages`
    and the documented query idioms cover them;
  - a `fields` parameter: rows are compact by default, with `raw` where
    detail matters (in aliases mode, `raw` rows are tokenized like any other;
    the name means full detail, not untokenized content);
  - a mailbox date range in `get_status`: `order: oldest` finds it;
  - `list_trash`: the CLI's trash listing carries no original path, so
    exclusions (R7) could not be checked without walking each item's parents;
  - `download_file` with several paths: the CLI names saved files itself
    (aliases mode has no `download_file`);
  - one search across mail, Drive and calendar;
  - exact cross-page thread collapse: it needs a per-query snapshot;
  - partial counts with a resume point: `before:` compares whole days;
  - manifests without the app's folder: that would walk the remote tree.

- Declined on 2026-10-04:
  - a `decode` command, an MCP Apps panel, a Proton Pass vault of aliases
    and a `reveal_entity` tool, as ways to show real names;
  - per-process numbered tokens, which change between processes;
  - a budget, or Touch ID, for searches by name;
  - tokenizing only private individuals, or keeping organizations in
    plaintext;
  - MCP elicitation as a gate, since Cowork does not render it;
  - a disk cache of summaries or detections, even a crypto-shredded one;
  - ranking a triage with a local model, since Claude could not see what it
    skipped;
  - format-preserving encryption, token vaults, realistic fake names,
    differential-privacy rewriting, homomorphic encryption and multiparty
    computation.

---

[← 9. Rollout and milestones](09-rollout.md) · [Contents](../rfc-0001.md#contents) · [11. Platforms →](11-platforms.md)
