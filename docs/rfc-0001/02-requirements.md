[RFC-0001](../rfc-0001.md) › 2. Requirements

# 2. Requirements (RFC 2119)

A requirement applies in both privacy modes unless it names one
([section 4, Settings](04-design.md#settings)). Where a requirement names a
macOS mechanism (the Keychain, `codesign`, LocalAuthentication, PDFKit, a
sandbox profile), [section 11](11-platforms.md) gives the Linux mechanism;
a feature that no Linux mechanism meets is absent on Linux, not weaker.

## By mode

| Area | Both modes | Aliases mode only | Withdrawn |
|---|---|---|---|
| Capabilities | R1 read-only | | R5 trash tools, R11 write switch |
| Secrets and network | R2 secrets, R3 network, R9 signature and pin | R20 privacy key | |
| Tool surface | R4 hints, R8 time limit, R26 the setting | R18 `reveal_*` behind Touch ID, R19 CLI | |
| Content | R6 hidden characters, R7 exclusions, R12 calendar freshness, R21 sandboxed converters | R13 pipeline, R14 aliases, R15 `entities`, R16 handles, R17 keyed digests, R22 no images, R23 guidance, R24 detectors | |
| Disk and logs | R10 in off mode: download and export folders; R25 logs | R10: no content on disk | |

## Requirements

- R1. protonctl MUST NOT implement any operation that sends data outside the
  account: no SMTP, no sharing, no invitations, no attendees. Since
  2026-10-04 it MUST NOT implement any operation that changes the account
  either: no drafts, labels, flags, moves, trash, uploads, folders or
  renames.
- R2. protonctl MUST NOT read, store, or receive the Proton account password or
  private keys. Its only secrets are the Bridge password, the calendar
  link(s) and, in aliases mode, the privacy key (R20), kept in the macOS
  Keychain or, on Linux, the Secret Service (Q31), and never logged or
  returned. In memory each is a `secrecy`
  secret type (`SecretString` for the password and the links): its `Debug`
  output is redacted (a test holds this), and its README says it is wiped
  on drop (not tested; [#12](https://github.com/matt-w-horn/protonctl/issues/12)). Copies
  made inside the Security framework, the `secret-service` crate and its
  D-Bus session, async-imap, rustls and curl's stdin pipe are beyond its
  reach.
- R3. protonctl's own sockets MUST go only to 127.0.0.1 (Bridge). The one
  internet request, the calendar feed GET, MUST run through `/usr/bin/curl`
  with the URL on stdin (never in argv), and only for an `https` URL whose host
  is `proton.me` or a subdomain of it. Partly machine-checked: `cargo deny`
  bans HTTP client crates, and a property test checks that every accepted link
  names a Proton host; the network sites are checked by reading them. The
  models of Phases 5 and 6 are files that ship with the install, pinned by
  SHA-256, and are never fetched at run time. Phase 5's model runs inside
  the server process (Q23); whether Phase 6's may run behind a socket on
  127.0.0.1, which this requirement would then have to name, is open
  (Q38).
- R4. Every tool MUST set `title`, `readOnlyHint`, `destructiveHint`,
  `idempotentHint`, and `openWorldHint: false` explicitly.
- R5. Withdrawn 2026-10-04: there are no trash tools (R1).
- R6. Third-party-authored content MUST be returned as JSON, named in a
  `provenance` member, with Unicode's default-ignorable characters (tag
  characters, bidi overrides, isolates and marks, zero-width spaces among
  them) and control characters other than tab and line breaks removed. The
  joiners, variation selectors and Hangul jamo fillers that emoji and some
  scripts are written with stay. Identifiers the model passes back (Drive
  paths until Phase 2, handles after it, R16) cannot lose characters, so
  every character a reader cannot see or type (those above, all controls,
  and whitespace other than the plain space) appears in a shown path as a
  visible `\u{...}` escape, and the tools and the CLI read an escape of any
  code point back as its character. A backslash that would begin an escape
  appears as `\u{5C}`, so a name containing an escape as text survives too.
  In aliases mode, aliases replace the names inside shown paths (R13).
- R7. Configured exclusions MUST apply to every read, the `reveal_*` tools
  included. An excluded item never appears in any result and reads as "not
  found".
- R8. Every tool call MUST finish within 150 s. A wait for user presence
  (R18) counts toward the limit; an unanswered prompt ends as
  `declined_by_user`.
- R9. Before every run of `proton-drive` (Q24), protonctl MUST verify that
  the binary is Proton's: on macOS by its code signature (Team ID
  `2SB5Z68H26`), and on Linux by the SHA-256 pinned at `setup drive`
  (Q33). It MUST check the CLI's version range once per process, and MUST
  pin Bridge's TLS certificate. The default path, `~/bin/proton-drive`, is
  one the user can write, so a binary swapped in between the check and the
  run is not seen; the check runs under the CLI's lock, just before the
  run, to keep that window short.
- R10. In aliases mode, from Phase 2, protonctl MUST NOT write message or
  file content, or text derived from it, to disk. Content goes from Bridge, the Drive app's
  folder or the CLI into memory, and leaves in a tool result.
  `proton-drive` cannot stream a file to stdout (Q14), so it downloads into
  a RAM disk that protonctl creates per process, readable only by the
  user, and detaches at exit (on Linux, `$XDG_RUNTIME_DIR`, checked as
  M2.8 says); M2.8 checks that the disk honours ownership,
  stays out of Finder and Spotlight, and that its pages reach only
  encrypted swap, and a read that cannot get such a disk fails. The export folder, the download folder,
  `download_file`, `export_drive_manifest`, the CLI's `drive manifest`,
  and the `export` and `inline` options of `get_attachment` and `drive
  get` are absent in aliases mode. The CLI writes a file only where its
  `--out` option names (`drive get` and `mail attachment` default it to
  the current folder), and from Phase 3 only after user presence (R19).
  R10 binds protonctl and the helpers it starts; the hosts' own
  transcripts and logs are outside it ([section 5](05-security.md)). In off mode, and in
  either mode until Phase 2 ships, the 2026-10-03 rule stands: content only
  in one 0700 per-process download directory, each download removed after
  an hour by wall clock and the directory deleted at exit, plus the export
  folder that `[export] folder` names, which MUST be absolute and outside
  the Proton Drive app's folder and the download cache, with any part that
  resolves outside it (a symlink) refused.
- R11. Withdrawn 2026-10-04: there are no writes (R1).
- R12. Calendar results MUST carry `fetchedAt` and the 8-hour freshness
  notice. The parsed feed is cached in memory for at most 15 minutes and never
  on disk. Freshness is judged by wall clock, `fetchedAt` against the current
  time, because `Instant` and tokio's timers stop while the Mac sleeps. The
  timer that frees the feed counts awake time only, so after sleep a stale
  feed can stay in memory until the timer fires, but it is never served.
- R13. In aliases mode, from Phase 2, every MCP tool result MUST pass the
  privacy pipeline ([section 6](06-privacy.md)) before it leaves the process: error results (`isError`)
  included, since errors quote paths, labels and IDs (`not found: <path>`),
  and every string the server returns, notes and page tokens among them.
  The pipeline fails closed: when any stage fails, the call returns an
  error that carries no content. Every detected person,
  organization, location, street address, email address, domain name, IP
  address, phone number, account number (card, IBAN) and URL is replaced
  by its alias (R14) wherever it appears: subjects, snippets, bodies,
  sender and recipient fields, `authentication` summaries and `count_messages`
  groups, file and folder names, labels, calendar titles, descriptions,
  attendees, organizers and locations, and local paths such as the Drive
  app's folder, whose name holds the account's address. Which of these
  each phase detects is listed in [section 6](06-privacy.md); from Phase 2 that includes
  US Social Security numbers, labelled one-time codes and passwords, and
  local account names (Q22).
  Dates, times and amounts stay plaintext. One exception: `reveal_*`
  results after user presence (R18), whose names stay as written while
  R16's handles and R17's keyed digests still apply. A name typed in the
  current call's query gets its alias in that call's result too; as
  typed, it appears only as a key of `queryEntities`, which maps it to its
  alias. Either pairing of a name with its alias holds in every transcript made under
  the same key ([section 5](05-security.md); Q21). No entity stays plaintext for being
  public: a minister and a major newspaper get aliases too.
- R14. From Phase 2, an alias MUST be three words from the curated word list
  ([section 6](06-privacy.md)), chosen by the first 8 bytes of HMAC-SHA-256 under the alias
  key, over a type tag and the canonical value, reduced modulo the
  cube of the list's length. The same value MUST give the same alias while
  the privacy key, the word list and the canonical form are unchanged, in
  every process and on every host, except where one result holds two
  entities that share an alias ([section 6](06-privacy.md)). protonctl MUST NOT store a map
  from aliases to values. Names enter the HMAC with one shared tag rather
  than their type; a fourth word is added only inside one result; and a
  change to the canonical form, the word list or a detector's typing
  raises the format version that `get_status` reports (Q19). URLs get no
  alias: each is `link N` within its result, beside its domain's alias.
- R15. From Phase 2, every result that contains an alias MUST carry an
  `entities` table with one entry per alias: its type, at most three hints,
  the aliases of any email addresses seen with it, and a `ref`. A `ref` is
  AES-SIV (RFC 5297) under the reference key, over the type and the
  canonical value padded to a multiple of 32 bytes, in base64url. Every
  query parameter that takes a name or an address MUST also take a `ref`. A
  `ref` that does not decrypt MUST be refused with an error that points to
  the `entities` table. A search by `ref` searches for the value it holds,
  and Bridge matches raw bytes, so a canonical value ("alice chen") misses
  "Chen, Alice" and "Zoë"; a `ref` holds the canonical value only, and a
  person's addresses carry their own `ref`s (Q19). The `entities` table counts toward
  the result's size, which Claude Code caps at 25,000 tokens ([section 6](06-privacy.md)).
- R16. In aliases mode, from Phase 2, results MUST NOT contain Proton's IDs
  (message, thread, node, revision, share), iCalendar UIDs, or RFC 5322 `Message-Id`,
  `In-Reply-To` or `References` values: each can link a transcript to the
  same item in another mailbox, calendar or Proton's records. They carry
  handles: AES-SIV under the handle key over the kind of item and its ID, in
  base64url. A Drive handle carries the path as stored, not the node UID,
  so exclusions (R7) still apply by path; it SHOULD be padded as a `ref` is,
  since its length otherwise gives the path's length, and it goes stale
  when the file or any folder above it is renamed or moved; the error
  then says to search again (Q20).
  Tools take handles and decrypt them, and check exclusions on the
  decrypted path; a handle that does not decrypt MUST be refused. Page
  tokens MUST be sealed the same way: today a Drive tree token carries
  its folder's path in hex, which any reader can decode, and a mail token
  carries IMAP UIDs.
- R17. In aliases mode, from Phase 2, results MUST NOT contain a raw digest. They carry keyed
  digests: HMAC-SHA-256 under the digest key over the algorithm's name and
  the raw digest, truncated to 64 bits and written as five words. The same
  function applies to a file's local SHA-256 and SHA-1 and to the SHA-1
  Proton claims, so an intact file's keyed local SHA-1 equals its keyed
  claim.
- R18. In aliases mode, from Phase 3, `reveal_message`,
  `reveal_attachment`, `reveal_file_content` and `reveal_event` (which off
  mode does not offer) each take exactly one handle and MUST NOT return
  anything before user presence: Touch ID, or the login
  password, through macOS's LocalAuthentication framework. protonctl writes
  the prompt text; it names the item and says that the item's full text
  will reach the model provider. An item's name (a subject, a file name) is
  written by a third party and could be phrased to make the prompt look
  harmless, so the prompt leads with protonctl's own words and facts it
  computes (kind, size, date), and shows the names people wrote, the
  item's and its folder's, last, each cleaned (R6), cut to 60 characters
  and in quotes. An approval covers that one
  item for 10 minutes by wall clock, so paging through it asks once. A
  refusal or a timeout returns `declined_by_user`; a prompt still open
  when the call's time limit (R8) ends is cancelled, and an answer after
  that approves nothing. The result carries an `entities` table that
  pairs each name in it with its alias (Q21). How
  protonctl reaches LocalAuthentication is open (Q15): the crate forbids
  unsafe code, so the options are a JavaScript for Automation script
  through `/usr/bin/osascript`, as PDFKit runs today, a signed Swift
  helper, or `objc2` bindings with `unsafe_code` lowered from `forbid` to
  `deny` and one reviewed exception.
- R19. In aliases mode, from Phase 3, CLI output passes the privacy
  pipeline by default. A `--raw` option, and every `--out` option, MUST
  require user presence as in R18, because a model with a shell can run the
  CLI ([section 5](05-security.md)). In off mode the CLI prints as built.
  `drive get` and `mail attachment` always write, so every run of them
  asks. Each CLI run is its own process, so its approval does not outlast
  it.
- R20. From Phase 2, the privacy key MUST be 32 random bytes in the
  Keychain (service `protonctl`, account `privacy-key`), in an item that
  does not synchronize, made by `protonctl setup privacy`. Subkeys for
  aliases, references, handles, digests and logs MUST come from HKDF-SHA-256
  with distinct labels. Keys MUST be held in secret types that redact
  `Debug` output and zeroize on drop, and MUST NOT appear in argv, the
  environment or the config. `protonctl rotate-key` replaces the key;
  afterwards every alias, reference, handle and keyed digest changes, and
  old references and handles are refused. A server already running notices
  the new key by a key ID, derived from the key, that `setup privacy` and
  `rotate-key` write into a non-secret attribute of the item, which reads without a
  Keychain prompt as `status` reads accounts, and loads it before its next
  result, so Claude Code and Claude Desktop never mix two keys (the item's
  modification date has whole-second resolution and could miss two changes
  in one second). The
  password calls protonctl makes today target the file-based login
  keychain, whose items iCloud does not sync, so "does not synchronize"
  holds without more (confirmed 2026-10-04: the `privacy-key` item is in
  `login.keychain-db`, the only keychain on the user's list;
  [#19](https://github.com/matt-w-horn/protonctl/issues/19)); but any program running
  as the user can read such an item after one Always Allow,
  `/usr/bin/security` included ([section 5](05-security.md)). Binding the item to user
  presence would need the data-protection keychain, and so an entitlement
  that an ad-hoc signed binary cannot carry (Q12).
- R21. From Phase 4, PDFKit, `textutil` and Vision run only in a child
  process under a sandbox profile that denies network access, Keychain
  access and file writes (Q13). They already run as child processes, with
  an empty environment, a 60 s limit and a 32 MiB cap on output:
  PDFKit through a fixed script in `/usr/bin/osascript`, and
  `/usr/bin/textutil`. Phase 4 either starts those under the profile or
  moves them behind `protonctl convert`, whichever Q13 shows works. On
  Linux, poppler and pandoc already run behind `protonctl convert`, under
  Landlock and seccomp (Q34).
  Bytes go in on stdin and UTF-8 text comes out on stdout. The parent
  process tokenizes the text (R13).
- R22. In aliases mode, from Phase 2, results MUST NOT contain image
  content, page images or file bytes in any encoding, `reveal_*` results
  included (Q42). Until Phase 4, an
  image or a scan returns its metadata and `"text": null` with the reason;
  from Phase 4 it returns the text Vision finds in it. Off mode keeps page
  images, image content and `inline` bytes as built.
- R23. In aliases mode, from Phase 2, a result MUST carry a short
  `guidance` line when its query named a person or an organization in
  plaintext, and when the result was cut short or paged; otherwise it
  carries none. Until Phase 5 a name
  is known in a query only as [section 6](06-privacy.md) describes, so the line can be
  missing for a bare name. The `reveal_*` tool descriptions say to try the
  tokenized reads first.
- R24. From Phase 2, every tokenized result MUST name the detectors that ran
  in a `detectors` member (`regex` and `dictionary` from Phase 2, `model`
  from Phase 5), so a reader can see when names in free text may have been
  missed.
- R25. From Phase 2, in either mode, logs on stderr MUST NOT carry
  content, names, queries, raw IDs or digests; protonctl's own log lines
  carry no identifiers at all, so none needs a key. A panic MUST print a
  fixed line, with at most the code location, and nothing of its message:
  Rust's own messages quote data (a slice off a character boundary prints
  up to 256 bytes of the string), and hosts keep stderr in log files.
  Children's stderr is never passed on.
- R26. From Phase 2, the privacy mode MUST be one setting, `off` or
  `aliases`, for every service and every host on the machine ([section 4,
  Settings](04-design.md#settings)), kept in the secret store as item
  `privacy-mode`, not in the config (Q28). Until the user sets it, there
  is no mode: every call MUST be refused with the two `setup privacy`
  commands, and `doctor` MUST fail (Q27). A server MUST refuse every call once the
  configured mode differs from the one it started in, and keep refusing
  even if the setting changes back, until the host restarts it. In
  aliases mode it MUST also refuse every call while the privacy key is
  missing, and it MUST NOT fall back to off or make a key by itself.
  `status`, `doctor`, `get_status` and the server's `instructions` MUST
  name the mode.

---

[← 1. Goals and non-goals](01-goals.md) · [Contents](../rfc-0001.md#contents) · [3. Options considered →](03-options.md)
