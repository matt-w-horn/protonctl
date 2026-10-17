[RFC-0001](../rfc-0001.md) › 10. Open questions

# 10. Open questions

Questions Q44 and later are in [RFC-0002](../rfc-0002.md#open-questions),
which continues this numbering.

| Q | Topic | State |
|---|---|---|
| Q1 | Calendar through a Full-view link; Contacts out | decided |
| Q2 | Open Bridge on demand | decided |
| Q3 | No decoded body scan | decided |
| Q4 | Run without Bridge, the Drive app and the link | answered |
| Q5 | Read-only | decided; [RFC-0002](../rfc-0002.md) proposes to amend it |
| Q6 to Q9, Q11 | The privacy layer's shape, entities, identifiers, calendar, rollout | withdrawn by Q43 |
| Q10 | Sandboxed converters | decided; narrowed by Q43 |
| Q12 | Signing identity and install path | decided |
| Q13 | `sandbox-exec` still enforced | decided |
| Q14 to Q16 | RAM disk, LocalAuthentication, token cost | withdrawn by Q43 |
| Q17 | Claude Code sandbox settings | decided |
| Q18 to Q23 | Word list, aliases, handles, pairing, entity types, the name model | withdrawn by Q43 |
| Q24 | Check the CLI's signature before every run | decided |
| Q25 | `/security-review` for Phase 2 | decided |
| Q26 to Q28 | The privacy setting, its default, where it lives | withdrawn by Q43 |
| Q29 | Linux is supported beside macOS | decided; development is on macOS |
| Q30 to Q34 | Linux: code layout, secret store, Bridge, Drive, converters | decided |
| Q35 | User presence on Linux | withdrawn by Q43 |
| Q36 | Listing in Anthropic's plugin directory | decided |
| Q37 | The Drive CLI's lock file stays in the cache folder | decided |
| Q38 to Q42 | Phase 6 runtime, second detector, phase order, `reveal_*` | withdrawn by Q43 |
| Q43 | Aliases mode withdrawn | decided |

Withdrawn questions keep their numbers, so code and issues that cite them
still point somewhere. Their full text, as decided, is no longer kept.

## Decisions and questions

- Q1, decided: Full-view ICS link, calendar read-only; Contacts
  out. Revisit if Proton ships a calendar or contacts API.
- Q2, decided: Bridge on demand. Its cold start is about 10 s to
  a working login, under the 20 s this RFC set, so when nothing answers on
  its port protonctl opens the Bridge app hidden
  (`open -g -j -b com.protonmail.bridge`) and retries for up to 40 s. It opens
  the app rather than the headless core, which would collide with the app
  the user opens themselves; a look-alike app gains nothing, since the
  certificate pin refuses it. Measured: with Bridge quit, a search took
  14.4 s and left Bridge running. `setup mail` never opens it.
- Q3, decided: no decoded scan. Bridge's raw-byte body search finds
  plain words in 1.5 s across the whole mailbox; the search tool's description
  says it can miss accented words and words split at line wraps.
- Q4, answered: run without Bridge, the Drive app and the
  calendar link? [Appendix B](appendix-b-proton-code.md) surveys Proton's code and lists the options.
  Decided: the client may handle the Proton password directly. But Proton's
  API refuses an honest third-party identity for Mail and Calendar
  at its first request (code 5002, tested without an account), so Bridge and
  the calendar link stay unless Proton grants one. Drive's third-party
  identity is accepted, as its SDK documents, and the Drive app is now
  optional (option 1).
- Q5, decided: read-only. Phase 1b, R5 and R11 are withdrawn, and
  R1 now forbids any change to the account. [RFC-0002](../rfc-0002.md)
  proposes to amend R1 for writes that a policy the user sets allows.
- Q10, decided and narrowed by Q43: document converters run in
  a sandboxed helper. The rest of Q10 (no content on disk, OCR, a VM
  helper) served aliases mode and went with it; the download and export
  folders stay (R10).
- Q12, decided: protonctl is signed with a self-signed
  code-signing certificate in the login keychain, made once by the
  maintainer, so a rebuild no longer brings a Keychain prompt and a
  swapped binary does. The binary stays in `~/.cargo/bin`. An Apple
  Developer ID waits until protonctl is distributed to others. Built as
  `scripts/install.sh`, which makes the identity on its first run, with a
  non-extractable key that no program is trusted to use
  (`security import -T ""`; without it `/usr/bin/security` itself is
  trusted), so each install asks before `codesign` signs: Always Allow
  there would let any process sign a swapped binary with no prompt, since
  any process can run `codesign`. The premise was checked in a throwaway
  keychain ([Appendix A](appendix-a-phase-0.md#signing-q12)). Read again
  for the plugin directory listing (Q36): the listing distributes source,
  not a binary, and `scripts/install.sh` makes the identity on each
  user's own Mac, so a swapped binary meets a Keychain prompt for each
  user.
- Q13, decided on macOS 27.0 (26A428), M4.1:
  `sandbox-exec` is deprecated, as its manual says, and still enforces.
  PDFKit through `osascript` and `textutil` run behind `protonctl convert`,
  as poppler and pandoc do on Linux, under one profile that
  `platform::macos::confine` builds with the reader's path and puts in
  front of the reader as `/usr/bin/sandbox-exec -p`; without `unsafe` the
  process cannot call `sandbox_init` itself. The profile is `(deny
  default)` plus the allows each reader was measured to fail without,
  found by failure, since denials are not logged (`log show` prints none,
  with `(debug deny)` too), then each removed in turn: exec of the reader
  alone; reads of `/`, `/usr/bin`, `/usr/share` and `/System/Library`, and
  the metadata of `/System` and `/System/Volumes/Data`; the `hw.`
  sysctls; and `/usr/lib`, which no fixture needed, kept for libraries
  outside the shared cache. Each allow's reason is on `READER_PROFILE` in
  `src/platform/macos.rs`. Nothing else is allowed: no file write
  anywhere, no network, no fork or exec of another program, no signal, and
  no Mach service, so no Keychain (`SecItemCopyMatching` returns -50 where
  it returns -25300 for a missing item outside), no Apple Events
  (`Application("Finder")` finds no application), no `doShellScript`, no
  `NSTask`, no WindowServer, font or preferences daemon, and no user font.
  `(subpath "/System")` was tried and does not reach the data volume
  through `/System/Volumes/Data`, since firmlinks resolve first; the
  narrower set is used anyway. Measured costs: a page image carries sRGB
  in place of the display's profile, the same to the eye; a PDF whose
  font is not embedded renders with a substitute; a PDF is parsed once
  per page image, as on Linux, where the one `osascript` run rendered
  four. `sandbox-exec` exits 65 for a profile that does not compile and 71
  for a reader it cannot run, and `convert::sandbox_failed` reads both as
  the sandbox's failure, as it does `convert`'s own 70. `textutil` exits 0
  and prints nothing for a file it cannot open, so a damaged document
  reads as empty text on macOS, where pandoc's failure is reported. Left
  open: the readers have no memory limit on macOS, where Linux has 2 GiB
  and pandoc's heap ([section 9](09-rollout.md#what-remains)); a
  crashed reader's state still goes to `~/Library/Logs/DiagnosticReports`,
  as macOS's crash reporter, not the reader, writes it.
- Q17, decided: the settings are in the README's
  [Claude Code sandbox](../../README.md#claude-code-sandbox) section.
  Checked on Claude Code 2.1.293 on macOS with a throwaway `--settings`
  file: a headless session ran each probe through its Bash tool, with
  the settings and without them, and the results were read from its
  tool results, not its summary. Probes printed exit codes and error
  text only.
  - The Drive app's folder: `filesystem.denyRead` on
    `~/Library/CloudStorage/ProtonDrive-*`. `ls` and a Python
    `os.listdir` fail with `Operation not permitted`; without the
    settings both succeed. The folder's name and existence stay
    visible: `stat` succeeds. The Read, Grep and Glob tools run
    outside the sandbox, so a `Read(...)` deny rule covers them: Grep
    is refused with "Permission to read ... has been denied", Read
    with "File is in a directory that is denied by your permission
    settings".
  - `proton-drive`: `denyRead` on the binary stops copying it (`cp:
    Operation not permitted`) but not running it: `proton-drive
    --version` exits 0 under the deny. Claude Code has no setting that
    denies execution. What cuts the CLI off is the network:
    `network.deniedDomains` with `proton.me` and `*.proton.me` refuses
    its connection, "deny network-outbound proton.me:443 (host is on
    the deny list)", even with `*.proton.me` in `allowedDomains`.
  - Bridge's port: nothing to add. With `allowLocalBinding` at its
    default `false`, a sandboxed command has no route to
    127.0.0.1:1143: `nc` exits 1 and `curl` 7, where without the
    settings `nc` connects. Claude Code sets `NO_PROXY` for localhost,
    so the direct connection stays blocked whatever `allowedDomains`
    holds, but a `localhost` or `127.0.0.1` entry there would open
    every local port to a command that goes through the proxy, so
    neither belongs in the list.
  - The config: `filesystem.denyWrite` on `~/.config/protonctl`, under
    which a Python `open(..., "w")` fails with `Operation not
    permitted`, and an `Edit(...)` deny rule for the Edit and Write
    tools and the file commands Claude Code recognizes, under which
    `touch` is refused before it runs. Reads stay allowed. A
    `Write(...)` rule is accepted and never consulted; `Edit(...)`
    covers both tools.
  - `security`: nothing to add, and no setting covers it. Inside the
    sandbox `security find-generic-password -s protonctl` answers "The
    specified item could not be found in the keychain" (exit 44);
    outside it finds the items (exit 0). That is the sandbox's default
    towards the Keychain on macOS, not a line in the settings, so it
    is checked again after each Claude Code upgrade;
    `excludedCommands`, `network.allowMachLookup: ["*"]` and
    `filesystem.disabled` would each reopen it.
  - Other hosts: `network.strictAllowlist: true` with `allowedDomains`
    as the list. `curl https://example.com` gets "deny
    network-outbound example.com:443 (host is not on the allow list)"
    (exit 56), with `--noproxy '*'` no route at all (exit 6), and
    with `example.com` allowed a 200. `allowUnsandboxedCommands:
    false` removes the retry outside the sandbox, and
    `failIfUnavailable` the silent fallback to no sandbox.
  Limits: the sandbox covers the commands Claude runs, not commands
  typed at `!`, `excludedCommands`, hooks or MCP servers, so protonctl
  itself runs outside it, as it must; the proxy decides by hostname
  without inspecting TLS; the permission rules match command text, not
  the program; Linux was not checked, and there is no Drive app folder
  there. The first attempt ran in auto mode, where the classifier
  refused several probes as credential exploration before the sandbox
  saw them; the recorded runs used `dontAsk`, so only the sandbox and
  the deny rules decided.
- Q24, decided: the Drive CLI's signature is checked before every run,
  one `codesign` call each, rather than once per process; this closes the
  window for a swap after the check and needs no admin-owned folder. A
  later review found the window still open: the CLI was checked, then run
  again by path. Now each call copies the CLI through one open of its
  path, checks the copy, and runs that copy for `--version` and the
  command: a sealed memfd on Linux, a file in a new 0700 folder on macOS.
- Q25, decided: `/security-review` runs on the Phase 2 changes, beside
  `/code-review`. It ran; its findings were in a security and privacy
  review that is no longer kept. A later assessment is in
  [docs/security-assessment.md](../security-assessment.md).
- Q29, decided: Linux is supported beside macOS on Apple
  silicon, so that protonctl builds and tests in cloud containers and
  serves Linux users. Windows stays out. A feature with no Linux mechanism
  that meets its requirement is absent on Linux ([section 11](11-platforms.md)).
  New work is now built and checked on macOS first; Linux keeps what is
  built, and its open work is tracked apart.
- Decided for Linux; the maintainer delegated each, and each gives its
  reason.
  - Q30: the module per system that P1 built (option B), with traits added
    when a fake or a second backend first needs one (C), and the one-line
    refusals as `cfg!` tests at their call sites (A).
  - Q31: the Secret Service over D-Bus (GNOME Keyring, KDE Wallet). Bridge
    and the Drive CLI need it on Linux anyway ([section 11](11-platforms.md#linux-availability-mp0-findings)).
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
    Decided again by the maintainer, from the review's finding: the pin
    lives in the Secret Service item `drive-cli-pin`, and not in
    the config, which the model can edit in Claude Code. The CLI's path
    stays in the config: the pin is of the file's content, so a path to
    other bytes is refused. Every `setup drive` asks on a terminal before
    it pins, the first one too, and `setup drive --cli <path>` asks and
    pins the CLI at a new path. `status` and `doctor` show the pin. A
    `cli_sha256` left in the config is ignored; every Drive call then says
    to run `setup drive`, which removes it. The store and the prompt stop
    a plain call through Claude Code's Bash tool, not every one: any
    program of the user can write the Secret Service once it is unlocked,
    and `script` fakes a terminal. macOS keeps no pin: its check is the
    signature.
  - Q34: external tools (`pdftotext` and `pdftoppm` from poppler-utils,
    and pandoc) inside `protonctl convert`, under Landlock and a seccomp
    filter. When a tool is missing, the result names the package to
    install. Built
    ([section 11](11-platforms.md#the-document-readers-as-built-p4)).
- Q36, decided by the maintainer: protonctl is listed in
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
    256 KiB or more. So the demo GIF is rendered small enough, and the
    MP4 stays out of the tree. `scripts/plugin-check.sh`, which
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
    tagged commit on `main` for each release. No release is tagged yet
    ([section 9](09-rollout.md#what-remains)).
- Q37, decided by the maintainer: the Drive CLI's lock file stays at
  `cli.lock` in the cache folder (`~/Library/Caches/protonctl`, or
  `~/.cache/protonctl` on Linux). It must be one path that every
  protonctl process finds, since it stops two servers (Claude Code and
  Claude Desktop, say) from running the CLI at once, which fails with
  "database is locked" (principle 6). The file is always empty, so R10
  holds.
- Q43, decided by the maintainer: aliases mode, the privacy
  layer of [section 6](06-privacy.md) as it was, is withdrawn, and off mode
  is the only behavior. The reasons:
  - It leaked by design. The name model found 94.6% of the people,
    organizations, projects, products and places on the synthetic corpus,
    so about one name in nineteen reached the host as written, and no
    detector stops a model from inferring who someone is from context.
  - It was most of the code: `src/privacy/` held 14,729 lines, its server
    tests 1,997 more, and the runtime comparison 7,886, with aliases-only
    branches in the server, the readers, the Drive and mail code and both
    platform files. Dropping its crates (`tract-onnx`, `tokenizers`,
    `aes-siv`, `phonenumber` and others) removed 132 packages from
    `Cargo.lock`.
  - Its user-facing cost: `setup privacy` before any call, a model to
    install, and no images, saved files or paths in aliases mode.

  What it took with it: the privacy key and the `privacy-mode` and
  `privacy-key` items, `setup privacy` and `rotate-key`, the per-call mode
  check (R26), aliases, refs, handles, sealed page tokens, keyed digests,
  the `entities` table, the name dictionary and the name model (Phase 5),
  the memory disk (Q14), OCR (M4.2, reachable only in aliases mode), and the
  planned `reveal_*` tools, user presence and local summaries (Phases 3, 6
  and 7). What stays: read-only tools (R1), exclusions applied before
  results exist (R7), hidden characters removed and `provenance` (R6),
  the call limit (R8), the download folder (R10), the sandboxed readers
  (R21, Q13, Q34) and the panic hook (R25). `doctor` names the two
  secret-store items and the name model's folder if an older version left
  them; `logout` deletes the items.

## Declined

- Declined:
  - per-label and per-folder scope levels: out of scope for this package;
  - downloading cloud-only files only to hash them: the SHA-1 Proton stores
    at upload serves joins with no download;
  - materializing Drive files in place: the export folder is the one
    outlet;
  - a log of every call;
  - batch search and `from_domain:` or `rcpt:` operators: `count_messages`
    and the documented query idioms cover them;
  - a `fields` parameter: rows are compact by default, with `raw` where
    detail matters;
  - a mailbox date range in `get_status`: `order: oldest` finds it;
  - `list_trash`: the CLI's trash listing carries no original path, so
    exclusions (R7) could not be checked without walking each item's parents;
  - `download_file` with several paths: the CLI names saved files itself;
  - one search across mail, Drive and calendar;
  - exact cross-page thread collapse: it needs a per-query snapshot;
  - partial counts with a resume point: `before:` compares whole days;
  - manifests without the app's folder: that would walk the remote tree.
- Declined with the privacy layer: ways to show real
  names, numbered tokens, budgets for searches by name, partial
  tokenization, elicitation as a gate, disk caches of detections, and
  heavier cryptographic schemes. The list's full text is no longer kept.

---

[← 9. Rollout and milestones](09-rollout.md) · [Contents](../rfc-0001.md#contents) · [11. Platforms →](11-platforms.md)
