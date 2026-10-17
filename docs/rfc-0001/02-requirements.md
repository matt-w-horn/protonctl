[RFC-0001](../rfc-0001.md) › 2. Requirements

# 2. Requirements (RFC 2119)

Every requirement applies to all of protonctl: there is one mode since
aliases mode was withdrawn ([Q43](10-open-questions.md)). Where a
requirement names a macOS mechanism (the Keychain, `codesign`, PDFKit, a
sandbox profile), [section 11](11-platforms.md) gives the Linux mechanism;
a feature that no Linux mechanism meets is absent on Linux, not weaker.

## By area

| Area | Applies | Withdrawn |
|---|---|---|
| Capabilities | R1 read-only | R5 trash tools, R11 write switch (Q5) |
| Secrets and network | R2 secrets, R3 network, R9 signature and pin | R20 privacy key (Q43) |
| Tool surface | R4 hints, R8 time limit | R18 `reveal_*` behind Touch ID, R19 CLI, R26 the setting (Q43) |
| Content | R6 hidden characters, R7 exclusions, R12 calendar freshness, R21 sandboxed converters | R13 pipeline, R14 aliases, R15 `entities`, R16 handles, R17 keyed digests, R22 no images, R23 guidance, R24 detectors (Q43) |
| Disk and logs | R10 download and export folders, R25 logs | |

## Requirements

- R1. protonctl MUST NOT implement any operation that sends data outside the
  account: no SMTP, no sharing, no invitations, no attendees. Since Q5 it
  MUST NOT implement any operation that changes the account either: no
  drafts, labels, flags, moves, trash, uploads, folders or renames.
- R2. protonctl MUST NOT read, store, or receive the Proton account password or
  private keys. Its only secrets are the Bridge password and the calendar
  link(s), kept in the macOS Keychain or, on Linux, the Secret Service
  (Q31), and never logged or returned. In memory each is a `secrecy`
  secret type (`SecretString` for the password and the links): its `Debug`
  output is redacted (a test holds this), and it is wiped on drop (on
  Linux a test reads the freed memory). Copies made inside the Security
  framework, the `secret-service` crate and its D-Bus session,
  async-imap, rustls and curl's stdin pipe are beyond its reach. The
  privacy key that aliases mode kept beside them (R20) was withdrawn by
  Q43.
- R3. protonctl's own sockets MUST go only to 127.0.0.1 (Bridge). The one
  internet request, the calendar feed GET, MUST run through `/usr/bin/curl`
  with the URL on stdin (never in argv), and only for an `https` URL whose host
  is `proton.me` or a subdomain of it. Partly machine-checked: `cargo deny`
  bans HTTP client crates, and a property test checks that every accepted link
  names a Proton host; the network sites are checked by reading them.
- R4. Every tool MUST set `title`, `readOnlyHint`, `destructiveHint`,
  `idempotentHint`, and `openWorldHint: false` explicitly.
- R5. Withdrawn: there are no trash tools (R1).
- R6. Third-party-authored content MUST be returned as JSON, named in a
  `provenance` member, with Unicode's default-ignorable characters (tag
  characters, bidi overrides, isolates and marks, zero-width spaces among
  them) and control characters other than tab and line breaks removed. The
  joiners, variation selectors and Hangul jamo fillers that emoji and some
  scripts are written with stay only where such text uses them: one
  variation selector directly after a visible base character, U+115F only
  before a Hangul vowel and U+1160 only after a leading consonant. A run of
  selectors, which can carry hidden text, loses all but the first, and a
  selector with no base goes. Identifiers the model passes back (Drive
  paths) cannot lose characters, so
  every character a reader cannot see or type (those above, all controls,
  and whitespace other than the plain space) appears in a shown path as a
  visible `\u{...}` escape, and the tools and the CLI read an escape of any
  code point back as its character. A backslash that would begin an escape
  appears as `\u{5C}`, so a name containing an escape as text survives too.
- R7. Configured exclusions MUST apply to every read. An excluded item
  never appears in any result and reads as "not found".
- R8. Every tool call MUST finish within 150 s. Work that untrusted input
  can make long is bounded inside it: a calendar query expands at most
  50,000 occurrences across its feeds, or 10 s of work, and says when it
  stopped (`complete: false`); a thread is fetched at most 32 MiB at a
  time; the Drive CLI's `--version` is killed after 20 s.
- R9. Before every run of `proton-drive` (Q24), protonctl MUST verify that
  the binary is Proton's: on macOS by its code signature, a Developer ID
  Application certificate of Proton's team (Team ID `2SB5Z68H26`), and on
  Linux by the SHA-256 pinned at `setup drive` (Q33). It MUST check the
  CLI's version range once per process, and MUST pin Bridge's TLS
  certificate. The default path, `~/bin/proton-drive`, is one the user can
  write, so protonctl runs a private copy, checked under the CLI's lock as
  the last step before the command runs; the copy that `--version` ran
  from is never the one the command runs.
- R10. protonctl MUST write message or file content only in one 0700
  per-process download directory, each download removed after an hour by
  wall clock and the directory deleted at exit, plus the export folder
  that `[export] folder` names, which MUST be absolute and outside the
  Proton Drive app's folder and the download cache. Writes there go
  through folders opened one name at a time with no symlink followed, and
  files are created relative to the folder held open, never replacing one;
  a symlink or a hard link anywhere on the way is refused before anything
  is made. Downloads are staged in the private download directory and
  copied out. R10 binds protonctl and the
  helpers it starts; the hosts' own transcripts and logs are outside it
  ([section 5](05-security.md)). The stricter rule of aliases mode (no
  content on disk, and a memory disk for the CLI's downloads, Q14) was
  withdrawn by Q43.
- R11. Withdrawn: there are no writes (R1).
- R12. Calendar results MUST carry `fetchedAt` and the 8-hour freshness
  notice. The parsed feed is cached in memory for at most 15 minutes and never
  on disk. Freshness is judged by wall clock, `fetchedAt` against the current
  time, because `Instant` and tokio's timers stop while the Mac sleeps. The
  timer that frees the feed counts awake time only, so after sleep a stale
  feed can stay in memory until the timer fires, but it is never served.
- R13. Withdrawn (Q43): the privacy pipeline over every result.
- R14. Withdrawn (Q43): aliases from the word list.
- R15. Withdrawn (Q43): the `entities` table and `ref`s.
- R16. Withdrawn (Q43): handles in place of Proton's IDs, and sealed page tokens.
- R17. Withdrawn (Q43): keyed digests in place of raw digests.
- R18. Withdrawn (Q43): `reveal_*` tools behind user presence.
- R19. Withdrawn (Q43): CLI output through the privacy pipeline, `--raw` and `--out` behind user presence.
- R20. Withdrawn (Q43): the privacy key and `rotate-key`.
- R21. PDFKit and `textutil` run only in a child process under a
  sandbox profile that denies network access, Keychain access and file
  writes (Q13). Built for PDFKit, through a fixed script in
  `/usr/bin/osascript`, and `/usr/bin/textutil`: each runs behind
  `protonctl convert`, under `sandbox-exec` with a deny-default profile,
  with an empty environment, a 60 s limit and a 32 MiB cap on output.
  On Linux, poppler and pandoc run behind the same command, under
  Landlock and seccomp (Q34). Bytes go in on stdin and UTF-8 text comes
  out on stdout. OCR (Vision and Tesseract, M4.2) was withdrawn by Q43.
- R22. Withdrawn (Q43): no image content or file bytes in aliases mode.
- R23. Withdrawn (Q43): the `guidance` line.
- R24. Withdrawn (Q43): the `detectors` member.
- R25. From Phase 2, logs on stderr MUST NOT carry content, names,
  queries, raw IDs or digests; protonctl's own log lines carry no
  identifiers at all. A panic MUST print a
  fixed line, with at most the code location, and nothing of its message:
  Rust's own messages quote data (a slice off a character boundary prints
  up to 256 bytes of the string), and hosts keep stderr in log files.
  Children's stderr is never passed on. In `serve`, the libraries' log
  output is off, since one logged a refused request's method name, and an
  error that stops the server prints a fixed line with no path or address.
- R26. Withdrawn (Q43): the privacy setting and the per-call mode check.

---

[← 1. Goals and non-goals](01-goals.md) · [Contents](../rfc-0001.md#contents) · [3. Options considered →](03-options.md)
