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
| Q26 | The privacy layer is optional | decided 2026-10-04; proposal confirmed |  |
| Q12 | Signing identity and install path | decided 2026-10-04 |  |
| Q13 | `sandbox-exec` still enforced | open | Phase 4 |
| Q14 | Drive downloads to stdout, or a RAM disk | decided 2026-10-04 |  |
| Q15 | Reaching LocalAuthentication | open | Phase 3 |
| Q16 | Token cost of words and base64url | decided 2026-10-04 |  |
| Q17 | Claude Code sandbox settings | open | Phase 3 |
| Q18 | Word list licence and curation | decided 2026-10-04; licence text to check in M2.2 |  |
| Q19 | Alias input, word count, URLs, what a `ref` holds | decided 2026-10-04 |  |
| Q20 | Drive handles: path or node UID | decided 2026-10-04 |  |
| Q21 | Pairing names with aliases | decided 2026-10-04 |  |
| Q22 | Further entity types; dictionary scope | decided 2026-10-04 |  |
| Q23 | Model runtime for Phases 5 to 7 | open | Phase 5 |
| Q24 | Check the CLI's signature before every run | decided 2026-10-04 |  |
| Q25 | `/security-review` for Phases 2 and 3 | decided 2026-10-04 |  |
| Q27 | Mode of a new install | decided 2026-10-04 |  |
| Q28 | Where the mode lives | decided 2026-10-04 |  |
| Q29 | Linux is supported beside macOS | decided 2026-10-04 | |
| Q30 | How platform code is structured | decided 2026-10-04 |  |
| Q31 | The secret store on Linux | decided 2026-10-04 |  |
| Q32 | Starting Bridge on Linux | decided 2026-10-04 |  |
| Q33 | Drive on Linux: the CLI, its check, no app folder | decided 2026-10-04 |  |
| Q34 | Converters and their sandbox on Linux | decided 2026-10-04 |  |
| Q35 | User presence on Linux | decided 2026-10-04 |  |
| Q36 | Listing in Anthropic's plugin directory | decided 2026-10-05 |  |

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
  want a read-only stand-in for Claude's Google connectors without it. Its
  proposal ([section 4, Settings](04-design.md#settings)) was confirmed the
  same day: one mode, `off` or `aliases`, for every service and host; off
  mode keeps the 18 tools as built, with their parameters, page images,
  downloads and the export folder; a service is on only once set up,
  Drive through a new `setup drive` (M1.4), so an install whose config has
  no `[drive]` table loses Drive until `setup drive` runs, and `doctor`
  and `status` say so; a running server refuses calls after a mode change,
  and aliases mode refuses without its key (R26).
- Q29, decided 2026-10-04: Linux is supported beside macOS on Apple
  silicon, so that protonctl builds and tests in cloud containers and
  serves Linux users. Windows stays out. A feature with no Linux mechanism
  that meets its requirement is absent on Linux ([section 11](11-platforms.md)).

- Decided on 2026-10-04 for Phase 2 and the Linux phases. The maintainer
  answered Q27 and delegated the rest; each gives its reason.
  - Q27: a new install has no mode. Until `setup privacy` or
    `setup privacy --off` runs, every call answers with those two commands
    and `doctor` fails. The two mistakes cost different amounts: a wrong
    `off` sends names to the provider, where they cannot be recalled; a
    wrong `aliases` costs one command to undo. An install upgraded to
    Phase 2 has no mode either, so the maintainer's install chooses once.
  - Q28: the mode lives in the secret store, as item `privacy-mode` beside
    `privacy-key`, not in the config. In Claude Code the model can edit
    the config file unless the sandbox denies it (Q17), so injected text
    could turn the layer off there; a Keychain item changed from outside
    protonctl meets a prompt. `status`, `doctor` and `get_status` show the
    mode, which keeps it visible. A missing item reads as no mode (Q27),
    so deleting it refuses calls rather than falling back to off.
    `setup privacy --off` asks for confirmation on a terminal in Phase 2,
    and for user presence from Phase 3 ([API specification](lld-api.md#command-line)).
  - Q12: protonctl is signed with a self-signed code-signing certificate
    in the login keychain, made once by the maintainer, so a rebuild no
    longer brings a Keychain prompt and a swapped binary does. The binary
    stays in `~/.cargo/bin`. An Apple Developer ID waits until protonctl
    is distributed to others. Built 2026-10-04 as `scripts/install.sh`,
    which makes the identity on its first run, with a non-extractable key
    that no program is trusted to use (`security import -T ""`; without
    it `/usr/bin/security` itself is trusted), so each install asks before
    `codesign` signs: Always Allow there would let any process sign a
    swapped binary with no prompt, since any process can run `codesign`.
    The premise was checked in a throwaway keychain ([Appendix A](appendix-a-phase-0.md#signing-q12)).
    Read again on 2026-10-05 for the plugin directory listing (Q36): the
    listing distributes source, not a binary, and `scripts/install.sh`
    makes the identity on each user's own Mac, so a swapped binary meets a
    Keychain prompt for each user. The Apple Developer ID waits until a
    built binary is distributed.
  - Q14: `proton-drive` cannot write a download to stdout: its
    `filesystem download` takes remote paths and one local folder, and
    nothing else (the CLI's source, `cli/src/commands/fileSystem/commandFileSystemDownload.ts`,
    commit 28ac9cd of 2026-10-02). So in aliases mode a cloud-only file is
    read through a per-process RAM disk on macOS, and through
    `$XDG_RUNTIME_DIR`, a per-user memory file system, on Linux. M2.8
    checks that the RAM disk needs no admin, honours ownership and stays
    out of Finder and Spotlight; where it cannot be made, the read fails
    rather than touch the disk.
  - Q16: aliases and keyed digests stay words, since people and Claude
    read them in prose ([Appendix C](appendix-c-roleplay.md)), which a
    token count does not change; refs and handles are base64url already.
    The token measurement sets only the page-size cap (M2.4), with
    Anthropic's token-counting API when a key is at hand.
  - Q18: the EFF large word list, curated as [section 6](06-privacy.md#identifiers)
    says: loaded words, names, brands and words that name a kind of place,
    organization, role or relation dropped, at least 7,132 words kept.
    Its licence is Creative Commons Attribution 4.0 International, which
    EFF's copyright policy gives all its original material (checked on
    2026-10-04; the wordlist post names no licence of its own), and the
    README credits EFF.
  - Q19: names (person, organization, location) enter the alias HMAC with
    one shared tag, `name`, so a detector that retypes a name keeps its
    alias (Q8); other types keep their own tag. Three words, with a fourth
    only when two entities in one result share three. A change to the
    canonical rules, the word list or a detector's typing raises the
    format version (the `v1` in the key labels), which `get_status`
    reports and the release notes announce; there is no automatic
    rotation. URLs get no word alias and no `ref`: each is
    "link N" within its result, beside its domain's alias, because a
    newsletter's hundreds of tracking URLs would crowd out the content and
    each become an entity; to open a link, the user opens the item in
    Proton. A `ref` holds the canonical value only; a person's addresses
    are listed in the entity, each with its own `ref`.
  - Q20: a Drive handle holds the path as stored, padded. Exclusions then
    apply by path with no lookup; a handle goes stale when the file or a
    folder above it is renamed or moved, and the error says to search
    again. A node UID would need a CLI `info` per use where the app's
    folder cannot map it, 4.6 s each ([Appendix A](appendix-a-phase-0.md)).
  - Q21: `queryEntities` pairs a typed name with its alias only when the
    name matches an entity in the result. `reveal_*` results carry the
    pairing, since the user approved that item's text and Claude cannot
    connect it to other results without it; the cost, that the pairing
    outlives the chat, is accepted in the [review](security-privacy-review.md).
    No key epochs: `rotate-key` stays a command the user runs.
  - Q22: Phase 2 adds three entity types: one-time codes and passwords
    that follow a label such as "code", "password" or "PIN" on the same
    line (`secret`), US Social Security numbers that pass their format
    rules (`national_id`), and the local account name in paths
    (`account`). The name dictionary covers the whole process: every
    correspondent's display name from All Mail's ENVELOPE (1.9 s for about
    20,000 messages) and every calendar attendee, built on the first
    aliases-mode call and held in memory only. It finds a correspondent's
    name in Drive names, subjects and titles, the largest gap before
    Phase 5.
  - Q24: the Drive CLI's signature is checked before every run, one
    `codesign` call each, rather than once per process; this closes the
    window for a swap after the check and needs no admin-owned folder.
  - Q25: `/security-review` runs on the Phase 2 and Phase 3 changes,
    beside `/code-review`, since they add protonctl's own cryptography and
    user presence.
  - Q30: the module per system that P1 built (option B), with traits added
    when a fake or a second backend first needs one (C), and the one-line
    refusals as `cfg!` tests at their call sites (A).
  - Q31: the Secret Service over D-Bus (GNOME Keyring, KDE Wallet). Bridge
    and the Drive CLI need it on Linux anyway ([section 11](11-platforms.md#linux-availability-mp0-findings-2026-10-04)).
    With none running, protonctl refuses to store secrets and never writes
    them to a file. The crate is chosen in MP2.
  - Q32: Bridge runs before protonctl does; protonctl never starts it on
    Linux, and `doctor` says how to run it as a systemd user unit.
  - Q33: the CLI's SHA-256 is pinned at `setup drive` and checked before
    every run, as Q24 does with `codesign`, since Proton publishes no
    checksum or signature for it; each CLI update needs `setup drive`
    again. Drive lists through the CLI, with search off, until a Linux
    Drive app ships.
  - Q34: external tools (`pdftotext` and `pdftoppm` from poppler-utils,
    pandoc, and Tesseract for OCR) inside `protonctl convert`, under
    Landlock and a seccomp filter. When a tool is missing, the result
    names the package to install.
  - Q35: polkit (`pkcheck --allow-user-interaction`) where an
    authentication agent runs, as in a desktop session; elsewhere Linux
    has no `reveal_*` tools.

- Q36, decided 2026-10-05 by the maintainer: protonctl is listed in
  Anthropic's plugin directory as a Claude Code plugin
  ([section 3, Packaging](03-options.md)). The plugin is the repository
  root. Its `plugin.json` declares one MCP server, which runs
  `scripts/serve`, a launcher that starts
  `~/.cargo/bin/protonctl serve`; the plugin holds no binary and no
  skills.
  - The name: the plugin's `name` is `protonctl`.
  - The licence: Apache-2.0.
  - The files: the directory stops validating a plugin folder that holds
    any file of 5 MiB or more, and holds for a reviewer any file other
    than an image or a font of 256 KiB or more. So the demo GIF is
    rendered at 8 frames per second and 720 pixels wide (4.1 MiB), and
    the MP4 stays out of the tree. `scripts/plugin-check.sh`, which
    `scripts/check.sh` runs, checks those limits on the index, that the
    plugin's version equals the crate's, and `claude plugin validate`.
  - The binary: users install protonctl themselves with `scripts/install.sh`,
    which builds it from source and signs it with a self-signed identity
    made on their own Mac (Q12). A Developer ID package is deferred. Each
    user builds and signs their own binary, so Q12's property holds for
    each user: a binary that something else swaps in meets a Keychain
    prompt. Neither this nor a Developer ID package covers the
    data-protection keychain.
  - Releases: the directory tracks a `release` branch, fast-forwarded to a
    tagged commit on `main` for each release.

- Open, to check before the phase that depends on each:
  - Q13 (Phase 4): `sandbox-exec` is marked deprecated in its man page.
    Check that it still enforces a profile on the current macOS, and that
    Vision, PDFKit through `osascript`, and `textutil` run under a profile
    that denies network access and file writes.
  - Q15 (Phase 3): does a LocalAuthentication prompt appear when protonctl
    runs as a child of Claude Desktop, as a child of Claude Code, and inside
    Claude Code's sandbox? Which route reaches it with unsafe code
    forbidden: an `osascript` script, a Swift helper, or `objc2` with one
    reviewed exception (R18)? Whichever starts the prompt is the process
    macOS names in it.
  - Q17 (Phase 3): which Claude Code sandbox settings deny reading the
    Drive app's folder, running `proton-drive`, connecting to Bridge's
    port, writing protonctl's config, running `security` on protonctl's
    Keychain items, and reaching hosts outside an allowlist?
  - Q23 (Phase 5): the model runtime for GLiNER, the Phase 6 local model
    and Privacy Filter: in process (`ort`, unsafe code in a dependency
    only), inside the sandboxed converter, or a local server on 127.0.0.1,
    which R3 would have to name and `deny.toml`'s HTTP-crate bans would
    have to allow; weights shipped with the install and pinned by SHA-256;
    their licences.

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
