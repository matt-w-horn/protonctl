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
| Q18 | Word list licence and curation | decided 2026-10-04; licence checked 2026-10-04 |  |
| Q19 | Alias input, word count, URLs, what a `ref` holds | decided 2026-10-04 |  |
| Q20 | Drive handles: path or node UID | decided 2026-10-04 |  |
| Q21 | Pairing names with aliases | decided 2026-10-04 |  |
| Q22 | Further entity types; dictionary scope | decided 2026-10-04 |  |
| Q23 | One model finds names (Otter), run by `tract` | decided 2026-10-05; runtime proposed |  |
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
| Q37 | The Drive CLI's lock file stays in the cache folder | decided 2026-10-05 |  |
| Q38 | Runtime for the Phase 6 model | open | Phase 6 |
| Q39 | No second detector in Phase 7 | decided 2026-10-05 |  |
| Q40 | Build Phase 5 before Phases 3 and 4 | decided 2026-10-05 |  |
| Q41 | No Claude Code prompt on `reveal_*` beside Touch ID | decided 2026-10-06 |  |
| Q42 | A reveal returns text, never page images | decided 2026-10-06 |  |

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
  with regex and a name dictionary, and a model after (Q23); services
  need not fail closed during the move. Local summaries stay, after the
  model.
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
    The review of 2026-10-06 found the window still open: the CLI is
    checked, then run again by path ([#72](https://github.com/matt-w-horn/protonctl/issues/72)).
  - Q25: `/security-review` runs on the Phase 2 and Phase 3 changes,
    beside `/code-review`, since they add protonctl's own cryptography and
    user presence. The Phase 2 review ran on 2026-10-06; its findings are
    in the [security and privacy review](security-privacy-review.md#6-phase-2-security-review-q25)
    ([#15](https://github.com/matt-w-horn/protonctl/issues/15)).
  - Q30: the module per system that P1 built (option B), with traits added
    when a fake or a second backend first needs one (C), and the one-line
    refusals as `cfg!` tests at their call sites (A).
  - Q31: the Secret Service over D-Bus (GNOME Keyring, KDE Wallet). Bridge
    and the Drive CLI need it on Linux anyway ([section 11](11-platforms.md#linux-availability-mp0-findings-2026-10-04)).
    With none running, protonctl refuses to store secrets and never writes
    them to a file. MP2 chose the `secret-service` crate
    ([section 11](11-platforms.md#the-secret-service-as-built-p2)).
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
    names the package to install. Built 2026-10-05 without OCR
    ([section 11](11-platforms.md#the-document-readers-as-built-p4)).
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
  - The files: the directory does not take a plugin folder that holds any
    file of 5 MiB or more, or any file other than an image or a font of
    256 KiB or more. So the demo GIF is
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

- Q37, decided 2026-10-05 by the maintainer
  ([#26](https://github.com/matt-w-horn/protonctl/issues/26)): the Drive
  CLI's lock file stays at `cli.lock` in the cache folder
  (`~/Library/Caches/protonctl`, or `~/.cache/protonctl` on Linux), in
  both modes, though in aliases mode it is a new file in the home folder.
  It must be one path that every protonctl process finds, since it stops
  two servers (Claude Code and Claude Desktop, say) from running the CLI
  at once, which fails with "database is locked" (principle 6). The
  memory folder is per process, so a lock there would lock nothing across
  processes, and a shared place in memory would need a fixed RAM-disk
  path or `/tmp`, which other users of the Mac share. The file is always
  empty, so R10 holds, and the no-disk test allows it only while it is
  empty ([section 7](07-testing.md)).

- Q23, decided 2026-10-05 by the maintainer
  ([#24](https://github.com/matt-w-horn/protonctl/issues/24)): one
  model finds names in free text, Otter (`whoisjones/otter-cross-mmbert`
  at commit `8729188`; Apache-2.0; 0.3B parameters on mmBERT-base; more
  than 100 languages; the entity types are plain words given at run time).
  The runtime, `tract`, a pure-Rust ONNX runtime in the server process,
  is proposed with this decision from the measurements below. A proof of
  concept measured both on 2026-10-05 on an M2 Pro with 16 GB
  (`scripts/otter-poc.py`, `scripts/otter-ocr-poc.py`):
  - Names corpus (`tests/fixtures/names.jsonl`: 57 synthetic texts,
    136 mentions, 14 languages), labels person, organization, project,
    product and location: recall 0.978 and precision 0.985 at a
    threshold of 0.2; 0.882 and 0.984 at 0.3; 0.993 and 0.964 at 0.1.
    Recall counts a mention found when every letter of it lies in a
    predicted span of any type.
  - OCR (`tests/fixtures/ocr-docs.jsonl`: 16 synthetic documents in six
    languages, rendered as scans, faxes and phone photos): on Apple
    Vision's text, recall 0.924 and precision 0.890 at 0.2; on
    Tesseract's, 0.906 and 0.871. Joining lines before detection lowered
    Vision's recall to 0.861, so text goes in as OCR gives it. On faxes,
    OCR garbled 12 of 21 names past recognition before the model saw
    them.
  - Runtimes (`scripts/runtime-bench`), on one window of 604 tokens,
    each checked against PyTorch's logits (largest difference under 1e-4, the same decisions
    at 0.2): `tract` 397 ms on five threads; ONNX Runtime through `ort`
    521 ms on five threads; `burn` 4.1 s on the CPU, and on Metal it
    stopped on an unsupported data type. `tract` and `burn` are pure
    Rust; `ort` links a C++ runtime, and its default build downloads it
    with `ureq`, which `deny.toml` bans. ONNX Runtime with six and eight
    threads took 4.1 and 5.7 times as long as with four, so the thread
    count is set, not left to the runtime.
  - The `tokenizers` crate (0.22, with `fancy-regex` and without its
    HTTP features) gives Python's token IDs and offsets for all 57
    texts. The `fix_mistral_regex` option transformers suggests for this
    tokenizer dropped recall to 0.331, so the tokenizer stays as shipped.
  - A model is configuration, not code: an ONNX file and a tokenizer,
    pinned by SHA-256, with its labels and threshold. A new model
    replaces it when the evaluation's recall per entity type and
    language does not fall. A change of model changes some aliases, so
    it raises the format version (Q19). The weights ship with the
    install (1.2 GB in float32) and are never fetched at run time. The
    detector is named `model` in `detectors`, since the model will
    change.
  - The dictionary's name rules (short forms, initials, misspellings,
    and an organization named by its own domain; D2 to D4 and D7) stay
    beside the model. One goes only when the evaluation (D1) shows that
    nothing it finds passes raw without it, per entity type and language
    (recorded 2026-10-06,
    [#53](https://github.com/matt-w-horn/protonctl/issues/53)).
  - Both corpora were written the day they were measured, and are small,
    so these numbers are optimistic. Phase 5 sets the threshold and the
    labels on a larger corpus: `product` gave most of the false positives
    ("caulk clear", "smoke detectors"). It also decides whether projects
    and products share Q19's name tag, and whether int8 or float16
    weights cut the size and the 3.6 s a 20,000-character page is
    estimated to take.

- Q39, decided 2026-10-05 by the maintainer: Phase 7 has no second
  detector; OpenAI's Privacy Filter is dropped. A study across 32
  benchmarks (arXiv 2608.02616) found its F1 0.40 on person names and
  near zero on non-Latin scripts (0.04 on Arabic), and on the SPY
  benchmark it scored below GLiNER2-PII (arXiv 2605.09973). It labels
  private individuals only, where Q7 tokenizes every entity. As a check
  after Otter it would add a second model to every page, its load and
  its time, for names Otter already finds.

- Q40, decided 2026-10-05 by the maintainer: Phase 5 is built before
  Phases 3 and 4. It depends on neither. It closes the residual the
  [security and privacy review](security-privacy-review.md) rates "high
  until Phase 5", names that only free text carries, and with it
  defects D5 and the second half of D7
  ([#4](https://github.com/matt-w-horn/protonctl/issues/4),
  [#43](https://github.com/matt-w-horn/protonctl/issues/43)). The phase
  numbers stay as they are.

- Decided on 2026-10-06 for Phase 3; the maintainer delegated both to the
  RFC's existing rules
  ([#25](https://github.com/matt-w-horn/protonctl/issues/25)).
  - Q41: `reveal_*` does not set
    `_meta["anthropic/requiresUserInteraction"]`. Q6 made Touch ID the
    gate because a host prompt can be answered with "Always allow", and
    because Claude Desktop and Cowork need the same gate as Claude Code; a
    second prompt in one host adds nothing to that. It would also undo
    R18's 10-minute approval, under which paging through an item asks
    once, by asking on every page, and a prompt on every call teaches the
    user to approve without reading. The tools still match no allow rule,
    so Claude Code asks before their first use as for any tool it has not
    been told to allow.
  - Q42: a reveal returns text, never page images or image content, after
    Touch ID as before it. R22 holds in aliases mode without exception.
    R18's prompt says that the item's full text will reach the model
    provider, and the `entities` table pairs the names in that text with
    their aliases (Q21); neither can describe a page image, which carries
    what no detector reads: faces, signatures, handwriting and the names
    in them. A scan or an image becomes readable through `reveal_*` in
    Phase 4, as the text Vision finds in it. A user who wants page images
    uses off mode (Q26).

- Open, to check before the phase that depends on each
  ([#21](https://github.com/matt-w-horn/protonctl/issues/21), [#22](https://github.com/matt-w-horn/protonctl/issues/22) and [#23](https://github.com/matt-w-horn/protonctl/issues/23)):
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
  - Q38 (Phase 6), split from Q23 on 2026-10-05: the runtime for the
    Phase 6 local model. That model writes text rather than labelling it,
    so Q23's span runtime need not fit it.

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
