[RFC-0001](../rfc-0001.md) › 7. Testing

# 7. Testing

Built (on macOS on 2026-10-08, `cargo test` ran 276 tests: 264 unit, 12
against the built binary, and 8 ignored by default: one lists Drive
through the real CLI, one is a timing, one reaches Dovecot, three run as
the child of another test, and two are the name model's measurements
(`measure`, `threshold_sweep`), run by hand in a release build. On Linux on 2026-10-07 it ran 268: 254 unit, 14
against the built binary; 12 more are ignored by default: one lists Drive
through the real CLI, one is a timing to run with `--release`, four reach
a Secret Service and one reaches Dovecot, which `scripts/check.sh` runs
in a throwaway keyring or container, and five run as the child of
another test. The macOS-only tests do not run on Linux, and the
Linux-only tests do not run on a Mac). Live,
`scripts/live-check.py` runs every tool once over MCP, as the Claude app
does, against the real calendar link, Bridge and Drive, and prints counts
only. On 2026-10-03 all 15 tools then built passed; its provenance check
caught a `read_file_content` reply for a file too large to return as text
that carried no `provenance` (R6), now fixed and covered below. Later that
day all 17 passed again, with the checks added since: 16-character
messageIds and no `encryption` in search rows, snippets, digests on saved
files, `count_messages`, an export into the export folder, and a complete
manifest with its SHA-256, each removed afterwards. On 2026-10-04 it
passed in aliases mode on a Mac, all 16 tools of that mode, and later
that day in off mode, all 18 tools (M1.2 in [section 9](09-rollout.md)).
The tests:

- Calendar: feed parsing (folding, escapes, VALARM isolation, a VEVENT without
  UID skipped); a weekly series across the DST change with an EXDATE and a
  moved occurrence; an all-day weekly series with a DATE `UNTIL` in a named
  zone, whose occurrence on the 25-hour day still ends at midnight; all-day and
  cancelled events; hidden characters removed from every invitation field, UID
  included; durations, including ones too long to represent; link host checks
  (`evilproton.me`, credentials, injected config lines, percent-escaped and
  full-width host characters); page tokens that carry their window, so a
  window anchored to "now" cannot shift between pages; three time formats.
  Property tests: parsing and expanding any feed-like text never panics (a
  planted panic on `FREQ=SECONDLY` was found and shrunk), and an accepted
  link's host, read independently, is `proton.me` or a subdomain (it found
  that `https://proton.me::` passed; a port must now be digits). Each of
  these was shown to fail with the code it replaced: a time outside the
  years 1900 to 2200, from the caller, a page token or an eventId, is
  refused rather than panicking chrono (`DateTime + TimeDelta` overflowed);
  a cached feed is dropped after 15 minutes even if nothing asks again
  (R12); and an all-day or floating occurrence keeps one eventId in every
  time zone (Paris and Los Angeles gave different IDs).
  - A cached feed is served only while its `fetchedAt` is under 15 minutes
    old by wall clock. An entry fetched 16 minutes ago whose `Instant` says 1
    minute, as after the Mac slept, is fetched again (shown to fail with
    freshness taken from the `Instant`, as before). A fetch time in the
    future is not fresh (shown to fail with the age's sign dropped).
  - HTML in a DESCRIPTION becomes text at parse time, so a search for `href`
    or `br` finds nothing while the link text still matches; plain text with
    a lone `<` comes through unchanged. Shown to fail with the HTML kept, and
    with any `<` taken as HTML.
  - `search_events` requires every word and quoted phrase, each in the title,
    description or location, and leaves out events matching a `-` term;
    `-Lunch -"Focus time"` alone returns every other event in the window. Shown
    to fail with the whole query matched as one substring (the old
    behavior), with negative terms ignored, with any one positive term
    enough, with a negative-only query refused, and with phrases split into
    words.
  - `get_event` finds a single event by its UID, one occurrence of a weekly
    series by `UID|<time>`, and a moved occurrence by the time it replaced;
    snapshots pin each `event` (shown to fail with single events marked
    recurring).
  - An empty result over a defaulted window carries a `note` naming the
    window; a window the caller chose, or a result with events, carries none.
    Shown to fail with the note missing from `list_events` or from
    `search_events`, and with it added for a chosen window or a result with
    events.
- Drive: a symlink lists as `kind: "symlink"` with no size (shown to fail:
  a live test saw `kind: "file"` with the 43-byte size of the link text);
  exclusions and hidden names skipped by search; excluding `/` hides
  everything; excluded paths read as not found, including through a symlink
  that points into an excluded folder; an excluded folder is not listed through
  a symlink to its parent; hidden characters in names shown as escapes and
  accepted back; a file too large or not text still names its provenance
  (shown to fail first); names with leading or trailing spaces kept; case- and
  normalization-insensitive matching and globs; `..` and symlink escapes
  refused; a property test that any accepted path is rooted, free of empty
  and dot parts, and comes back unchanged after being shown escaped (it
  failed once the backslash escape was undone).
- Reading content inline (`src/extract.rs`), against macOS's own helpers:
  text is read as UTF-8, UTF-16 with a byte-order mark, or else Windows-1252,
  flagged as a guess (a 27-byte Latin-1 file was refused in a live test),
  a UTF-8 byte-order mark over a Latin-1 byte included;
  a NUL byte, many control codes, or a byte-order mark followed by an odd
  number of bytes mean binary (that last case was caught by an older test,
  where a 3-byte binary file read as empty UTF-16 text); PNG, JPEG, GIF and
  WebP are known by their first bytes; a PDF made by `cupsfilter` comes back
  as its text through PDFKit, and a file that only claims to be a PDF does
  not; a .docx made by `textutil` comes back as its text. Text pages follow
  on by `offset` and `nextOffset` to the end, an offset past it is refused,
  and hidden characters go before offsets are counted, so pages line up;
  a document whose pages hold no text says so. A PDF's pages come as
  images, rendered by PDFKit to JPEG with a 2,000-pixel long edge, 4 per
  call, each after a "Page N:" label, with `nextPage`: when `page` asks for
  them, and unasked when the PDF has no text layer. A live check found four
  of six PDFs sampled were scans, whose text was only the page separators.
  A six-page PDF comes as pages 1 to 4 and then 5 and 6; a page past the
  end, page 0, and page on a text file are refused; a scan made by turning
  a rendered page back into a PDF with `sips` comes as its page image
  (shown to fail with scans returned as empty text, as before)
  (a 100,000-character page was measured at 110,496 characters of JSON,
  over Claude Code's limit, so a page now defaults to 20,000).
  - Through Drive: a 55,000-character file reads in pages; a Latin-1 file,
    a PDF and a PNG each come back (the PNG as image content); with
    `inline: true` a file's bytes follow the JSON as an embedded resource
    and nothing is saved. The second page of a cloud-only file needs no
    second download (shown to fail with the last document not kept).
  - Through the scripted Bridge: an attachment that is not readable text is
    saved and its path returned, and asked for inline its bytes come back
    instead, with no path.
- Drive downloads, against a stand-in CLI script: the exact arguments;
  nothing from the environment but `HOME` and the log level reaches it (shown
  to fail without `env_clear`); a hidden character in the name saved as an
  escape; a failed item refused although the CLI exits 0; folders and
  excluded paths refused before the CLI runs; a name the CLI changes
  (`Notes: Q3.txt` saved as `Notes_ Q3.txt`) found as the one file in a new
  folder, and a second copy refused rather than replacing the first (both
  shown to fail before the fix); parts shaped like a node UID, and folder
  names ending in a backslash, refused before the CLI runs (shown to fail
  before the fix).
- Drive without the app's folder, against the same stand-in answering
  `list` and `info`: a listing with excluded children left out and names
  that cannot be paths (undecryptable, or holding a `/`) named in `unlisted`; an
  excluded and a missing path giving the same `not found` (shown to fail
  when the CLI's wording goes unrecognized), the excluded one without a CLI
  call; search refused; a read sized from `info` then fetched, and a file
  too big refused before any download; a name stored decomposed sent back as
  stored (shown to fail when normalized to NFC). The path property covers
  both the NFC form and the as-stored form.
- Drive identity and digests, against the same stand-in with node and
  revision IDs and a claimed SHA-1:
  - Entries from the CLI carry `nodeId`, `revisionId`, `claimedSha1`,
    `claimedModified` and Proton's `modified`, and a claim that is not 40
    hex digits comes back null; entries from the app's folder name their
    time `localModified` (shown to fail with the old fields, and with any
    claim passed on as text).
  - A CLI listing names each entry it cannot show in `unlisted`: a name
    holding `/` escaped as stored, an undecryptable one with a null name
    (shown to fail with the old `skippedNames` count). When an exclusion
    names a child of the folder, undecryptable entries are only counted in
    `unlistedHidden` (shown to fail with that check off). Folder mode
    counts the dot-names it leaves out in `hiddenNames`, excluded ones not
    included (shown to fail with none counted, and with excluded ones
    counted).
  - `get_file_metadata` with `digests` in folder mode adds Proton's view
    from one `info` call, the local file's SHA-256 and SHA-1, and whether
    the claim matches, both ways; a failing CLI gives `protonError` beside
    the local facts, and a file over 1 GiB is not hashed (each shown to
    fail with its part removed).
  - Saved downloads carry the SHA-256 and SHA-1 of the stand-in's `hello`,
    and `matchesClaimedSha1` true or false from the CLI's claim (shown to
    fail without digests and without the claim). The hasher matches known
    digests and reads files across chunk boundaries (shown to fail with
    the match flag always true, and with SHA-256 in place of SHA-1).
  - A cloud-only read deletes its fetched copy at once (shown to fail when
    it stayed until exit).
  - In aliases mode a cloud-only read goes through the memory disk (M2.8):
    the CLI writes into a folder on a mounted RAM disk, the content comes
    back, and the folder and then the disk are gone (shown to fail with
    the download folder used, as before, which aliases mode refuses).
    macOS only; Claude Code's sandbox refuses the RAM disk.
  - B21 (D8): a folder made online-only in the app, whose listing is not
    on this computer, is listed through the CLI with a note, or, when the
    CLI cannot list it, comes back empty with a note saying its contents
    are not here; in a tree its row carries `cloudOnly: true` (shown to
    fail with no folder taken as online-only, the empty listing with no
    note that D8 found). The test stands in a folder for a dataless one,
    a flag only the system sets. Built:
    `a_folder_only_in_the_cloud_is_listed_through_the_cli_or_said_to_be`
    in `src/drive/mod.rs`.
  - Two handles on the CLI lock file exclude each other, and a CLI run
    waits while another holds it (shown to fail with no lock taken, and
    with the run not taking it).
- The Drive CLI's check (R9, Q24, Q33,
  [#72](https://github.com/matt-w-horn/protonctl/issues/72)), against
  stand-in CLIs. Every run is of the private copy, on Linux a sealed memfd
  run as `/proc/self/fd/N`. A `#!/bin/sh` script cannot run from there,
  since the fd is close-on-exec and the shell cannot open that path, so
  each stand-in is one `#!/usr/bin/env -S /bin/sh` line that runs its
  script, and the trusted stand-ins of the Drive tests run through the
  copy too.
  - A pinned CLI runs only while it matches its pin, checked before every
    run, with `--version` once per process; one with no pin, or a pin for
    other bytes, never runs.
  - A CLI swapped in after the check never runs. The stand-in renames
    another CLI over itself while it answers `--version`, between the
    check and the command, as the security review's stand-in did; the
    command still runs the checked bytes, the next call refuses the
    swapped file, and on Linux every run is of `/proc/self/fd/N` (shown
    to fail with the command run by path, as before #72, and with only
    `--version` run by path). Built:
    `a_cli_swapped_in_after_the_check_never_runs` in `src/drive/cli.rs`.
  - A `cli_sha256` in the config is not trusted, even when it matches the
    file: with no pin in the secret store the CLI never runs, and the
    error says the pin moved and to run `setup drive` on a terminal; the
    same pin from the store is trusted (shown to fail with the config's
    pin taken when the store has none or cannot be read). Linux only.
    Built: `a_pin_in_the_config_is_not_trusted` in `src/drive/cli.rs`.
  - A first `setup drive --cli <path>`, against the built binary, refuses
    without a terminal: it names the file and its SHA-256, runs nothing,
    and writes no config (shown to fail with a first setup that pins
    without asking, which ran the stand-in). Linux only. Built:
    `tests/setup_drive.rs`.
  - `status` shows the pin on Linux, or why there is none, and nothing on
    macOS; `get_status` in aliases mode leaves it out, with nothing in
    `dropped` (shown to fail without its field policy, which named it in
    `dropped` on every call). `setup drive --cli` on a set-up Drive changes only `cli` and
    removes a `cli_sha256` left from before, comments and every other
    value kept. The copy hashes each byte it writes, across chunks.
  - Not automated: the whole flow on a pseudo-terminal from `script`, in a
    throwaway keyring, on 2026-10-07: a first setup confirmed with `yes`
    stored the pin and wrote only `cli`; `status` and `doctor` showed the
    pin; a piped `setup drive --cli` was refused, and on a terminal it
    pinned the new CLI and removed an old `cli_sha256`. The CLI runs on
    Bun ([Appendix A](appendix-a-phase-0.md)), likely as a Bun standalone
    binary, which reads its own code from its executable: one built with
    Bun 1.4.2 ran from a sealed memfd and found its code. The real CLI, on
    a Linux machine signed in to Proton, has not run this way
    ([#62](https://github.com/matt-w-horn/protonctl/issues/62)).
- Surface: no forbidden tool names; every hint set and every description
  within 2,048 characters; a snapshot of the whole tool surface, descriptions
  included, since Cowork re-approves a tool whose definition changes;
  misspelled parameters refused (the test failed first: `start_time`
  deserialized to an empty window).
- Mail, against a fake Bridge with its own self-signed certificate: the first
  connection records the certificate and logs in; the pinned certificate is
  accepted and any other refused (shown to fail with the pin check disabled);
  a refused login, a plain-text (STARTTLS-mode) greeting and an unanswered
  port each give a plain message, the last as the typed cue that makes
  protonctl open the Bridge app; fingerprints round-trip through hex. These
  tests bind a local port, which Claude Code's sandbox refuses by default.
- Mail operations against a scripted Bridge over TLS that answers LOGIN,
  LIST, EXAMINE, STATUS, UID SEARCH and UID FETCH: on one session,
  `search_threads` (default, `has:attachment`, and `includeTrash` with
  snippets), `count_messages` by sender, `get_message` on a row's
  messageId, `get_thread` and `list_labels`, over an All Mail holding a
  thread, an import's second copy, a message with an attachment and one
  also in Trash, beside a `\Noselect` parent and one that refuses STATUS.
  Snapshots pin each result and every command sent. Shown to fail with the
  Trash and Spam pass returning no IDs. A `label:` search leaves out a
  labelled message that is also in Trash, as the tool description says
  (shown to fail when only All Mail searches left Trash and Spam out).
- Mail search and reading: the Gmail-style subset translated to IMAP SEARCH
  (operators, dates, negation, OR, IMAP quoting, UTF-8 terms); an OR beside
  `in:`, `label:` or `has:` refused rather than joined to an earlier term;
  `has:attachment` kept out of SEARCH and tested on each summary; property
  tests that `compile` never panics and never lets a line break into the
  criteria (it found that `newer_than:300000y` panicked inside chrono), and
  that IMAP quoting reads back exactly; threads named by their root message,
  skipping Bridge's own `protonmail.internalid` entry, and found when
  References has no space between IDs (shown to fail: such a message had no
  threadId); a message rendered with
  its body and attachments, hidden characters removed from its subject. Each
  of these was shown to fail with the code it replaced: a call that fails or
  is cut off mid-command leaves no session for the next one; two attachments
  with the same index and name never overwrite each other, and a file
  already in `--out` is never replaced; the thread search names every header
  a thread's root is read from, `In-Reply-To` included.
- Mail paging, against pure functions:
  - A property test pages through any mailbox, in either order, while a
    newer message arrives or an unseen one goes between pages; every hit
    shows exactly once. It was shown to fail with the offset tokens earlier
    builds handed out, where an arrival repeated a row, and with the
    cursor's own hit counted again.
  - After Bridge renumbers a mailbox (a new UIDVALIDITY), the next page
    restarts at the cursor's second, repeating rows rather than skipping
    them (shown to fail with UIDVALIDITY ignored). A token from the other
    order is refused.
  - Tokens round-trip, and anything else, the old offset form included, is
    refused (shown to fail with trailing parts accepted).
  - `label:` finds folders as well as labels, nested ones written as
    `list_labels` shows them, and asks for a prefix when a label and a
    folder share a name (shown to fail when only labels matched).
  - The any-recipient idiom the query description documents,
    `to:x OR cc:x OR bcc:x`, compiles to a chained OR.
- Mail search rows, from synthetic header blocks through the code that
  shapes live summaries:
  - Thread mates merge into the first row of their thread on a page, and
    copies into the row holding the original, so a thread with an import's
    duplicate shows once with `matching` and `copies`. Shown to fail when
    copies were keyed on `Message-Id` alone, which also swallowed a
    different message reusing that ID, and when messages without a thread
    key shared one empty key.
  - A hit that needs a new row on a full page comes back for the next page.
  - Rows leave out `encryption`, false flags and empty fields, and list 3
    To addresses plus a count of the rest (shown to fail with `encryption`
    kept).
  - A Trash copy stays out by its full ID although rows show 16 characters
    (shown to fail when the check used the shown ID).
  - A messageId matches a stored ID by a prefix of 16 characters or more;
    shorter ones are refused (shown to fail with any prefix accepted).
- Mail counts, against the pure tally `count_messages` reports:
  - An import's second copy counts once.
  - Address case is ignored, and a To address listed twice in one message
    counts once.
  - Subdomains stay separate groups, and a message with no To address
    counts under `(none)`.
  - The three orders sort as named.
  - Shown to fail with copies counted again, with case kept, and with the
    per-message duplicate counted twice.
- Mail reading, against pure functions and synthetic headers:
  - Plain-text bodies lose quoted replies: lines starting with `>` and the
    attribution before them (Gmail's two-line form included), and an
    Outlook header block or "-----Original Message-----" with everything
    after it, except in a forward. Inline answers stay, and so does a line
    that merely ends in "wrote:". Shown to fail with attributions kept,
    with `>` lines kept, and with forwards cut.
  - HTML-only bodies lose every `blockquote`, the `gmail_quote` and
    `protonmail_quote` containers (nested ones included), and Outlook's
    reply header onward (shown to fail with nesting ignored).
  - `authentication` summarizes the topmost Authentication-Results header,
    the receiving server's, so a header the sender added below it no
    longer shows. Shown to fail with the last copy, which mail-parser's
    `header_raw` returns and earlier builds showed.
  - Copies in a thread merge into the first, and their hidden characters
    do not count (shown to fail with merging off).
  - A long thread comes in pages of about 30,000 characters of body text,
    at least one message each, where up to 50 messages of 8,000 came back
    in one result before (shown to fail with every message on one page).
  - Snippets, asked for with `snippets: true`, come from the header block
    and the first 32 KiB of the body: the plain alternative of a
    multipart message, HTML without its styles or quoted part even when
    cut mid-document, quoted-printable, and base64 cut mid-quad. Quoted
    replies go, whitespace collapses, and the result stops at 200
    characters. Shown to fail with quotes kept, with whitespace kept, and
    with no cap.
- CLI: IDs and queries that start with `-` parse as values, and `-h` alone
  still prints help.
- The export folder, against a temporary tree and the stand-in CLI:
  - A download with `export: true` lands at `drive/<Drive path>`, and a
    second one is refused rather than replacing the first (shown to fail
    when the export saved into the private folder).
  - An exported attachment is saved once and then returned as it is (shown
    to fail when it was overwritten). Other bytes or a symlink at its name
    are refused rather than returned as the attachment with its digests
    (shown to fail when whatever was there came back).
  - A manifest lists the tree without excluded paths and reports its
    SHA-256, and without the app's folder it is refused rather than walking
    the remote tree. Its rows come in listing order: each folder's entries,
    then its subfolders' in turn.
  - `list_drive_tree`, three rows a page, so a folder's entries span pages,
    returns exactly the manifest's rows across its pages (shown to fail when
    a page lost its place inside a folder: the pages never ended).
  - A `withSha1` manifest built one folder per call equals one built in a
    single call. It carries the stored SHA-1, names what Proton lists but
    the app's folder does not show, and leaves out an excluded folder the
    CLI lists. Shown to fail with the time budget ignored, and with the
    excluded name written.
  - In a `withSha1` manifest, a node whose name does not decrypt gets a row
    naming its `parent`, unless an exclusion names a child of that folder
    (shown to fail with that check off).
  - A page token names the folder in progress and how many of its rows
    were returned, and each call finds the folders after it from the disk
    around it, without walking the whole tree; plain manifests page the
    same way. A manifest goes on in a new `Drive`, as in another process,
    after the folder the token names is deleted and a new one arrives after
    it (shown to fail when every call walked the tree and a changed tree
    refused the token). A token goes on only with the folder and
    `withSha1` it began with: one passed for another folder, or with the
    other `withSha1`, is refused (shown to fail when only a folder count was
    compared); malformed tokens are refused.
  - A forged token naming an excluded folder, or a symlink into one, never
    lists what is inside it; one naming a folder outside the manifest's, or
    holding a `..` part, is refused (shown to fail with the token's folder
    read without `resolve`).
  - A `withSha1` call that fails partway writes no rows, so its retry with
    the same token repeats none (shown to fail with rows written as each
    folder was listed).
  - A manifest asked for through a symlink to a folder leaves out that
    folder's excluded child (shown to fail when the walk named entries by
    the path asked for, R7).
  - The folder must be absolute and outside the Proton Drive app's folder,
    including through `..`, and outside the download cache; a part swapped
    for a symlink that leads out is refused. Shown to fail with each check
    removed. A folder not made yet is checked where it will be, so one
    reached through a symlink to the Drive folder, or spelled in another
    case, is refused (shown to fail when such a path was checked as text).
- Shutdown, against the built binary with a throwaway home: closing stdin,
  SIGTERM, SIGHUP and Ctrl-C each delete the download folder (R10), and the
  server exits within 10 s each way (SIGHUP shown to fail without its
  handler). Shutting the runtime down ends a CLI still running, so it cannot
  write into the folder after it goes (shown to fail without
  `kill_on_drop`). With `RUST_LOG=debug` set, a tool result still never
  reaches stderr, which hosts keep in log files (shown to fail when the
  filter came from `RUST_LOG`: rmcp logged the whole result). Closing stdin
  before initialize, which the Claude app does to a copy it starts and
  drops at once, is a clean stop rather than an error (shown to fail: exit
  1 with `connection closed: initialize request`).
- Downloads expire: a download folder is expired after 61 minutes by wall
  clock but not after 59, and after a jump like a night's sleep; a creation
  time in the future is not expired (shown to fail with no expiry, as before,
  with a 10-hour life, and with a future time counted as expired). The sweep
  removes an old `drive-` folder and keeps a new `mail-` folder, an old
  folder with another name, and a `drive-` symlink to an old folder outside
  the download folder (shown to fail with the age ignored, with any name
  swept, and with links followed). `get_status` counts this process's
  downloaded files and bytes, subfolders included (shown to fail with the
  field absent, as before, and with subfolders skipped).
- Content: tag and bidi characters removed, ZWJ emoji kept; zero-width
  characters, soft hyphen, LRM, ESC and BEL removed, while tab, line
  breaks, a no-break space, ZWNJ and an emoji's variation selector stay; in
  names, 15 characters a live test found shown raw (zero-width, bidi marks,
  no-break and narrow spaces, U+2028, U+180E, U+3164, U+FE0F, U+008F) are
  escaped and typed back, and an escape of any code point is read (each
  shown to fail with only tag and bidi characters handled); property tests
  that escaping round-trips any text, escapes written out literally included
  (the property found that a literal `\u{202E}` came back as the character),
  and that `clean` removes exactly the hidden characters; truncation on
  character boundaries; page tokens, including one too large to add to.
  Config: the `[[calendar]]` entry that setup appends reads back with its name
  exact, and a config that cannot be read is left as it is rather than
  replaced by the new entry (shown to fail with the read error taken for an
  empty file).

Each guard was first shown to fail on a constructed bad input: the
forbidden-name check on a list containing `send_message`; the exclusion test
against the same tree without the exclusion; the snapshot after renaming one
tool title; the HTTP-crate ban after adding `ureq`. The symlink test found a
real gap (exclusions were checked only on the requested path) and failed until
`resolve` also checked the resolved one. The tests added in the 2026-10-02
review each failed on the code before its fix. The OR test failed on the code
before its fix, and the CLI test reproduced the `-`-led thread IDs that failed
in a live run. The earlier test of the `has:attachment` translation passed
while the live search matched every message ([Appendix A](appendix-a-phase-0.md)): only a live
comparison against each row's attachment count caught it. The SIGTERM test
failed on the code before its fix, and the first fix, which returned through
tokio's runtime, failed the test's exit deadline instead: tokio's stdin reader
cannot be cancelled, so `serve` now exits directly once the folder is gone.
The lint table rejected a planted `dbg!`, `println!`, `todo!` and an
`#[expect]` that matched nothing in `serve.rs`, and probe crates (Rust
1.99.0) passed clean and failed on each planted defect, for the lint table and
`deny.toml` alike; an MPL-2.0 dependency failed the license check. The redaction test failed with a `String` standing in for the secret,
and the coverage floor failed when set to 99%.

Planned for the privacy layer, each shown to fail on a constructed bad input
before it is trusted. Each item says which of its tests exist on
2026-10-05, by function name; [the issues labeled tests](https://github.com/matt-w-horn/protonctl/issues?q=label%3Atests) list the
missing ones:

- Settings (Phase 2): a `privacy-mode` item holding neither `off` nor
  `aliases` refuses every call; aliases mode with
  no privacy key refuses every call and makes no key; a server refuses
  every call once the configured mode changes, in either direction, and
  keeps refusing when it changes back; with no mode set (Q27), every call
  gives `privacy_mode_unset`;
  `status`, `doctor`, `get_status` and the `instructions` name the mode;
  off mode passes the 2026-10-03 tests unchanged; Drive is off without a
  `[drive]` table and on with one. Each shown to fail with its check
  removed. Built: `no_mode_refuses_every_call`,
  `a_mode_change_refuses_until_restart_even_when_changed_back`,
  `aliases_mode_refuses_without_a_key_and_never_falls_back`,
  `an_unreadable_key_is_not_a_missing_one` and
  `an_unreadable_setting_refuses_in_either_mode` in `src/privacy/mod.rs`;
  `calls_are_refused_without_a_mode_or_after_a_change` and
  `get_status_in_aliases_mode_shows_no_local_path` in `src/serve.rs`;
  `drive_is_on_only_once_set_up` in `src/main.rs`; and
  `status_doctor_and_the_instructions_name_the_mode` in `src/main.rs`,
  which checks that `status`, `doctor` and the `instructions` name the
  mode, or say there is none or that it cannot be read.
- Leak test, end to end (Phase 2, in aliases mode): the scripted Bridge, the stand-in CLI and
  a synthetic calendar feed hold a planted corpus: names in headers and in
  bodies, in Latin and other scripts; email addresses; phone numbers from
  several countries; card numbers that pass the Luhn check; IBANs; street
  addresses; URLs; domains; Proton IDs; digests; names in subjects, file
  names and event titles. Every tool runs, errors and page tokens
  included (a missing path, a bad label, a refused handle), and the test
  fails if any planted value, compared after NFKC and case folding, appears
  in any result other than a `reveal_*` result, or inside a `ref`, a handle
  or a page token, decoded from base64url and hex. Shown to fail with the
  pipeline bypassed for one tool, and for error results. Parts of planted
  values (a given name or surname alone, an address's local part or
  domain) and encoded forms (quoted-printable, HTML entities,
  percent-escapes) are searched for too: they fail the test where a
  detector claims them, and are listed in the report otherwise. The
  report lists what each detector caught, so names that appear only in
  free text show as the known Phase 2 gap. Built:
  `no_planted_value_leaves_any_tool` in `src/serve_every_tool.rs` plants
  names in four scripts, addresses, phone numbers from four countries,
  cards, IBANs, links, Proton and IMAP IDs and digests in the scripted
  Bridge, a calendar feed, the Drive app's folder and a stand-in Drive
  CLI, makes 75 calls over all 16 aliases-mode tools through two servers,
  and searches every key and string of every result, as text and as what
  each base64 or hex run decodes to, for each value whole, in parts and
  in canonical form. A name typed in a query may stand only as a key of
  `queryEntities`. Two values it knows pass are listed, and asserted to
  still pass, so the test fails when the gap closes: a name only in free
  text, and a street address. Its first run found two leaks, both fixed:
  a Chinese one-word name was never found, and a page asked to start
  inside a name showed its tail.
  `every_aliases_tool_is_called_through_the_server` checks that those
  calls cover every tool each server lists (I9). Smaller tests plant
  values in one result: `a_search_result_keeps_no_name_id_or_number` and
  `people_in_events_become_aliases` in `src/privacy/pipeline.rs`, and
  `aliases_mode_returns_aliases_and_handles_only` in `src/serve.rs`.
- Stability: the corpus, run in two processes under one key, gives the same
  aliases, references, handles and keyed digests; under another key, all
  of them differ. Built:
  `identifiers_are_stable_under_one_key_and_all_differ_under_another` in
  `src/privacy/pipeline.rs` runs a corpus of five results holding 19
  kinds of identifier (an alias of each entity type, refs, each kind of
  handle, page tokens and keyed digests) under keys derived again from the
  same bytes, then in a second process that the test starts, and then
  under another key, where none of the first run's identifiers appears.
  At the parts: `aliases_are_stable_words_and_names_share_a_class` in
  `src/privacy/ident.rs`, `subkeys_differ_and_are_stable` in
  `src/privacy/key.rs`, and
  `the_same_value_has_the_same_alias_in_every_result` in
  `src/privacy/pipeline.rs`.
- Round trips: property tests that any value's `ref` decrypts to that value
  and any ID's handle to that ID, and that a changed byte, a foreign key or
  garbage is refused (R15, R16). Built: the property tests
  `any_ref_or_handle_opens_to_its_value_and_nothing_else` (every handle
  kind, also refused as another kind) and `garbage_opens_as_nothing`, and
  with fixed values `refs_handles_and_tokens_round_trip_and_refuse_tampering`
  and `padding_keeps_trailing_zeros`, all in `src/privacy/ident.rs`.
- Keyed digests: an intact file's keyed local SHA-1 equals its keyed claim,
  and one changed byte makes them differ. Built:
  `keyed_digests_still_compare` in `src/privacy/pipeline.rs`, with the
  claim in upper-case hex; and `digests_are_keyed` in
  `src/privacy/ident.rs`, where another key or another algorithm gives
  another keyed digest. B24 (built 2026-10-04): a 40- or 64-hex digest
  and a Proton message ID written in free text become a keyed digest
  and a message handle, and a longer run of either is left alone. Built:
  `digests_and_message_ids_in_text_are_found` in
  `src/privacy/detect/pattern.rs` and
  `digests_and_message_ids_in_text_are_rewritten` in
  `src/privacy/pipeline.rs`, each shown to fail with the two patterns
  matching nothing.
- Canonical values: case, diacritics, honorifics and "Last, First" forms
  give one alias; canonicalizing twice changes nothing (property test). And
  the other way: names that differ only by a Devanagari or Thai mark, "M.
  Chen" and "Mme Chen", "Mr Chen" and "Ms Chen", and "John Smith Sr." and
  "John Smith" keep different aliases; so do names that start with a
  word that is an honorific in one language and a name or an initial in
  another: "Pan Wei Ming" and "Wei Ming", "Sri Mulyani Indrawati" and
  "Mulyani Indrawati", "M. J. Smith" and "J. Smith", "Dame Babacar Diop"
  and "Babacar Diop", "Sig Ole Hansen" and "Ole Hansen" (#74, each shown
  to fail with the old honorific list). Built: `spellings_of_one_name_meet`,
  `rules_never_merge_two_people` (every case above) and the property test
  `canonical_forms_are_fixed_points` in `src/privacy/canon.rs`.
- Collisions: with a word list of 4 words, two and three entities that
  share an alias in one result all get distinct aliases, in an order that
  does not depend on the order of the input. Built:
  `entities_sharing_three_words_get_distinct_aliases_in_any_order` in
  `src/privacy/pipeline.rs` finds two addresses and three addresses that
  share their first three words under a 4-word list, which test builds put
  in place with `words::with_list`, and runs all five in ten orders.
- Size: a result whose `entities` table holds 500 URLs stays under Claude
  Code's 25,000-token limit, its page shortened to fit. Replaced as built:
  URLs get no entity (Q19), and a result over the cap of 90,000
  characters is refused, not shortened
  ([As built](lld-privacy-layer.md#as-built));
  `a_result_over_the_cap_is_refused` in `src/privacy/pipeline.rs` tests
  that.
- Errors and tokens: every error a tool can return passes the pipeline,
  and a page token carries no name, path or UID in any encoding (R13, R16).
  Built: `errors_and_page_tokens_carry_no_name_path_or_uid` in
  `src/serve_every_tool.rs` makes 20 calls built to fail, across the
  tools, and checks that each error is exactly a code, its fixed message
  and `detectors`, and that every page token holds no planted value,
  decoded or not, and opens as its tool's sealed kind; opened, a mail
  cursor holds an IMAP UID and a tree token a folder named after a
  person, so the sealing is what keeps them out. Also
  `aliases_mode_errors_are_codes_without_the_path` and
  `aliases_mode_refuses_a_value_that_is_not_a_handle` in `src/serve.rs`,
  and `local_paths_and_errors_become_fixed_text` in
  `src/privacy/pipeline.rs`.
- Fail closed: a pipeline stage made to fail returns an error with none of
  the planted values. Built: in test builds, text holding
  `pipeline::PLANTED_PANIC` panics the rewrite stage, after the names and
  register stages have run, and
  `a_pipeline_stage_that_panics_fails_closed` in `src/serve.rs` reads a
  file holding it with an address and a phone number: the result is
  exactly `pipeline_failed`, with none of them. A panic before the
  pipeline, in the operation, is answered with `internal`
  (`an_operation_that_panics_in_aliases_mode_is_answered`).
- Page edges (built 2026-10-04, after the first live aliases-mode run): a
  file holding an address and a phone number, read 16 characters at a
  time in aliases mode, returns neither in part, and its last words
  still come back (shown to fail with pages cut as before: the first page
  ended `jane.do`, raw). `truncate` in aliases mode moves its cut off a
  mention, back to its start or past its end (shown to fail with the old
  `truncate`, which left `to ann`). These are
  `an_address_cut_by_a_page_edge_does_not_leak` in `src/serve.rs` and
  `aliases_mode_truncates_beside_a_mention` in `src/content.rs`. Added
  2026-10-06 (#69): a page edge at every byte inside "Dana Okonkwo", a
  name only the message's own From holds, so the process dictionary that
  cuts pages lacks it, leaves no part of it raw on the page before the
  edge or the page from it
  (`a_page_edge_inside_a_header_name_leaves_no_half_raw` in
  `src/privacy/pipeline.rs`); and a cut or a start inside a word moves to
  the word's start, or a cut past its end when the word starts the piece
  (`aliases_mode_cuts_between_words` in `src/content.rs`).
- Rotation: a server running while `rotate-key` replaces the key gives new
  aliases on its next call, and refuses the old handles. Built at the key:
  `a_rotated_key_is_picked_up_on_the_next_call` and
  `a_key_read_between_its_two_writes_is_used` in `src/privacy/key.rs`. At
  the server: `a_rotated_key_gives_new_aliases_and_refuses_old_handles`
  in `src/serve.rs` lists and reads a file, changes the key under the
  running server, and expects the new key's alias, a new `fileId`, and
  the old `fileId` and ref refused.
- Names in queries: a typed name gets its alias in that call's result, and
  appears as typed only as a key of `queryEntities`, paired with its
  alias. `a_name_typed_in_the_query_is_paired_with_its_alias` in
  `src/privacy/pipeline.rs` tests this on a search's `from` field.
- Guidance: present in exactly the cases R23 names. Built: after a name
  typed in a query (`a_name_typed_in_the_query_is_paired_with_its_alias`),
  and when a result has a page token to follow, or a page, a body or a
  Drive index marked as cut, with both lines together under 200
  characters (`guidance_comes_when_more_exists`, both in
  `src/privacy/pipeline.rs`).
- No disk, in aliases mode: every tool runs with a throwaway home and temporary folder,
  compared before and after; any new file fails the test (shown to fail
  against the 2026-10-03 download folder). The helpers run with a cleared
  environment, so they find the real home and the per-user folders
  (`getconf DARWIN_USER_TEMP_DIR` and `DARWIN_USER_CACHE_DIR`) through the
  system, not through `HOME`; the test also compares those, or runs each
  tool under a sandbox profile that denies and reports file writes. Built
  in part: `aliases_mode_gets_no_download_folder` in `src/content.rs`, and
  a cloud-only read through memory that leaves nothing behind,
  `an_aliases_mode_read_fetches_into_memory_and_leaves_nothing` on Linux
  and `an_aliases_mode_cloud_only_read_goes_through_the_memory_disk` on
  macOS, in `src/drive/mod.rs`. Over every tool:
  `aliases_mode_writes_no_file` in `src/serve_every_tool.rs` runs the
  test binary again with a throwaway home and temporary folder, makes
  every call, and fails on a new file in either or on anything left in
  the memory folder after a call. Beside it,
  `the_cli_in_aliases_mode_writes_no_file` runs the CLI's commands that
  could write, in aliases mode, the same way: `drive manifest` and the
  `--export` and `--inline` options of `drive get` and `mail attachment`
  are refused, a cloud-only `drive cat` reads through the memory folder,
  and `drive get --out` saves only where `--out` names. Before the fix
  for [#73](https://github.com/matt-w-horn/protonctl/issues/73), it
  failed on `drive manifest` and both `--export` options, which wrote
  into the export folder, and on `drive cat`, which made the download
  folder; the `--inline` options failed only on the refusal's wording,
  since the CLI's default `--out` already refused them. Both tests allow
  the Drive CLI's lock file while the file is empty, and the lock stays
  in the cache folder (Q37). On macOS they also allow the memory disk's
  mount point while it is empty, since it stays mounted until the process
  exits: run first on a Mac on 2026-10-06, the CLI test failed on that
  folder alone. On macOS `every_tool` reads a cloud-only file only in
  T7's child, since the RAM disk is one per process and the drive test
  detaches it. With a file planted on the disk at each cloud-only read,
  both tests failed: the CLI test on `drive cat` and `drive get --out`,
  T7 on `read_file_content`; before that read was added on macOS, T7
  passed with the plant. Neither watches the folders a reader finds
  through the system: on
  Linux the readers can write only to `/dev/null` (Landlock), and on
  macOS nowhere (`sandbox-exec`, Q13).
- No images or bytes, in aliases mode: no result carries image content, an
  embedded resource or base64 file data (R22). Built in part:
  `a_scan_in_aliases_mode_gives_a_reason_and_no_images` in
  `src/extract.rs`, and `aliases_mode_offers_no_saving_and_no_paths` in
  `src/serve.rs`, which finds no `inline` or `page` parameter. Over every
  tool: `no_tool_returns_images_or_file_bytes_in_aliases_mode` in
  `src/serve_every_tool.rs` checks that every block is text, that no
  result has structured content, and that no string decodes to a file
  signature or a planted file's bytes; a binary attachment comes back as
  a `reason`. An image file, a scan and an image attachment come back as
  a `reason` on macOS; on Linux (M4.2, 2026-10-10) each holds planted
  names and numbers as rendered text, comes back as the text Tesseract
  finds, and `no_planted_value_leaves_any_tool` finds those values only
  as aliases.
- Logs: with `RUST_LOG=debug`, stderr holds none of the planted values
  (R25), and a panic planted on a slice of planted text prints none of it.
  Built in another form: `results_stay_out_of_stderr_even_with_rust_log_set`
  in `tests/serve_exit.rs` looks for a phrase of `get_status`'s result,
  and `a_panic_prints_where_and_withholds_what` in `src/main.rs` and
  `a_blocking_panic_withholds_its_message` in `src/content.rs` plant a
  panic message.
- Surface: one snapshot per mode. The forbidden names gain the withdrawn
  write tools in both modes, and `download_file` and
  `export_drive_manifest` in aliases mode. Since a name list misses a new
  write tool named some other way, the test also holds each mode's exact
  set of tool names: in aliases mode every one with `readOnlyHint: true`;
  in off mode `readOnlyHint: false` on the three that can leave a file
  behind (`get_attachment`, `download_file`, `export_drive_manifest`;
  M1.6) and true on the rest. Built: `tool_surface_snapshot` and
  `aliases_tool_surface_snapshot`, `registered_names_are_the_tool_enum`
  (each mode's exact set), `aliases_mode_offers_no_saving_and_no_paths`
  (no `download_file` or `export_drive_manifest`, and `readOnlyHint: true`
  on every tool) and `no_tool_offers_a_forbidden_capability`, in
  `src/serve.rs`; the off-mode hints are in the snapshot. The forbidden
  names do not list the withdrawn write tools; the exact set keeps them
  out.
- User presence (Phase 3), through an injectable checker: a refusal returns
  nothing but `declined_by_user`; an approval covers one item for 10 minutes
  by wall clock, judged as R12 judges freshness; a prompt still open at the
  time limit is cancelled and a late approval covers nothing; a subject
  written as an instruction appears in the prompt last, cut and quoted; the
  CLI's `--raw` and `--out` ask as the tools do. Not built (Phase 3).
- Converter sandbox (Phase 4): a test build of the helper that tries to
  open a socket, write a file and read a Keychain item fails at each;
  Vision returns the words of a synthetic page rendered as an image.
  OCR on Linux (built 2026-10-10, M4.2): `each_reader_runs_in_its_sandbox`
  reads `tests/fixtures/ocr.png` through `convert ocr`, and
  `the_reader_holds_the_sandbox` checks that Tesseract holds the seccomp
  filters and limits and an environment of `OMP_THREAD_LIMIT=1` alone;
  `images_and_scans_are_read_by_ocr_in_aliases_mode` in `src/extract.rs`
  reads an image and an 11-page scan in aliases mode, and checks that
  the first 10 pages are read and the note says so.
  Built on Linux, ahead of Phase 4: `the_sandbox_confines_a_reader`, which
  runs `sandbox_probe` as a child, in `src/platform/linux.rs`, and
  `each_reader_runs_in_its_sandbox` and
  `a_damaged_pdf_fails_in_the_reader_not_the_sandbox` in
  `tests/convert.rs`. Built on macOS 2026-10-07 (M4.1, Q13):
  `the_sandbox_confines_a_reader` in `src/platform/macos.rs` runs this
  test binary again under `confine`, with only `sandbox_probe` selected,
  which finds TCP to a port the parent listens on, UDP, a Unix socket the
  parent listens on, a write to the temporary folder, to `/usr/bin` and to
  `/System/Library`, a read of the temporary folder and of the home
  folder, a chmod, a new process and a read of a program that is not the
  reader each refused with `PermissionDenied`, and a read of a Keychain
  item failed: an item the parent makes under the service
  `protonctl-test-sandbox-probe`, reads back outside the sandbox, and
  deletes after, never one of protonctl's. The two `tests/convert.rs`
  tests above run on macOS too, with each reader's own output (PDFKit
  puts no blank line before a form feed, `textutil` none between
  paragraphs), and `a_reader_the_profile_cannot_run_is_the_sandbox_s_failure`
  records `sandbox-exec`'s exits 71 and 65, which
  `a_sandbox_failure_is_told_from_a_reader_s` in `src/convert.rs` reads as
  the sandbox's. Each was shown to fail with the old behaviour planted:
  the probe with writes allowed (with the path lookups a write needs and
  no data reads, since without them the lookup fails first), with every
  Mach service allowed (the Keychain read succeeded), with the network
  allowed (TCP connected), and with fork, exec and `/usr/bin/true`
  allowed (the process started); `each_reader_runs_in_its_sandbox` with
  `/usr/bin` dropped from the profile, which keeps `osascript` from
  starting; the damaged-PDF test with the script exiting 70; and the
  exit-code test with `sandbox-exec`'s codes read as a reader's. Vision:
  not built (M4.2).
  Memory and signals (built 2026-10-07, #71): a 200 KB Word file holding
  200 MiB of XML stops with pandoc's "Heap exhausted" in about a second,
  where without the heap limit pandoc was still reading at 15 s; each
  reader holds two seccomp filters, 2 GiB of address space and a core
  size of 1, and pandoc a 1 GiB heap; a sandboxed process cannot signal
  another (`kill`, `tkill`, `tgkill`, `rt_sigqueueinfo`,
  `rt_tgsigqueueinfo`, `pidfd_open`, `pidfd_send_signal`, `kill(0, 0)` and
  `kill(-1, 0)`), set a file owner that would send it `SIGIO`, start a
  process or change a limit, yet can signal itself and start a thread.
  Built: `a_document_built_to_fill_memory_stops_at_the_heap_limit` and
  `the_reader_holds_the_sandbox` in `tests/convert.rs`, and
  `sandbox_probe` and `the_sandbox_lets_no_signal_out`, which runs
  `signal_probe` as a child and calls through perl what Rust cannot
  without `unsafe`, in `src/platform/linux.rs`. Each was shown to fail
  without its part of the fix: the memory test at its 15 s deadline
  without the heap limit, even with the address-space limit; the limits
  check without either limit; the probes without the new rules, and
  `sandbox_probe` without the `clone3` rule, which let a process start
  through `posix_spawn`. Run on Linux 6.18, where Landlock also scopes
  signals, so the kernels before 6.12 were tested only through the
  seccomp rules, which these tests check on any kernel.
  The x32 numbers (built 2026-10-10, #12):
  `the_filters_refuse_each_call_by_both_its_numbers` runs the two filters
  `sandbox` applies in the test process, through a small interpreter of
  the BPF instructions seccompiler emits, as seccomp runs them: over each
  call's `struct seccomp_data`, newest filter first, the first action in
  the kernel's order kept. Every refused call is checked by its x86_64
  number and its x32 number, with the arguments that refuse it and, for
  the calls refused only for some arguments, ones that are allowed;
  `clone3` gets `ENOSYS`, `getpid` is allowed, and a 32-bit call ends the
  process. The numbers are written out from the kernel's
  `syscall_64.tbl` (v6.18), not taken from `x32` or `refused_calls`; the
  test failed with the x32 rules removed, with `ioctl`'s x32 number
  mapped to its x86_64 one, with `truncate`'s rule dropped, and with
  `prlimit64`'s rule inverted. It checks the filter, not the kernel:
  that seccomp sees the x32 number before the kernel looks for an x32
  table is the kernel's ABI, seen once on Linux 6.18 without x32, where
  a perl probe got `EACCES` with the rules and `ENOSYS` without them.
- Secrets wiped on drop (R2, R20; built 2026-10-10, #12): on Linux,
  `secrets_are_wiped_on_drop` in `src/secret.rs` and
  `subkeys_are_wiped_on_drop` in `src/privacy/key.rs` drop a secret and
  the shared subkeys, then read the freed block through `/proc/self/mem`,
  so no unsafe code reads it, and expect zeros past the allocator's own
  16 bytes. Each first checks that a dropped plain `String` still shows
  its text, so an allocator that returns the block to the system cannot
  pass the test. With a plain `Box<str>` in place of the secret, and with
  the keys never dropped, each failed.
- Names in text (Phase 2): a known name is found wherever it stands
  alone, in any case and any script. B25 (built 2026-10-04): a Chinese or
  Japanese name inside running text in its own script, which has no
  spaces, is found, a two-character one too, while a Latin name inside a
  longer word is not. Built: `a_name_in_running_text_without_spaces_is_found`
  in `src/privacy/detect/dict.rs`, shown to fail with the boundary rule
  applied to every script. B15 and B16 (built 2026-10-04): a known
  person's given name, surname and name without middle names, and the
  initials of the result's own people, are found, a one-word form of a
  name the result does not hold only inside a sentence; a form joins its
  full name's alias only when one name fits and the result holds it, and
  is otherwise its own entity linked by `maybeSameAs` both ways. Built:
  `short_forms_and_initials_are_found` in `src/privacy/detect/dict.rs`,
  shown to fail with no short forms made, and
  `a_short_form_joins_its_full_name_only_when_one_local_name_fits` in
  `src/privacy/pipeline.rs`, shown to fail with joining off. B17 (built
  2026-10-04): a known name misspelled by typing or OCR is found as near
  it, and a name that differs more, another known name spelled right, or
  a run in lower case is not. Built:
  `misspelled_names_are_found_as_near_their_name` in
  `src/privacy/detect/dict.rs`, shown to fail with the misspelling pass
  off. B20, first half (built 2026-10-04): a sender that is an
  organization or a product is typed so when its name is its own
  address's domain, whole or as its first word, which is then its short
  form, and a person stays a person. Built: `organizations_that_send_mail_are_typed_organization` in
  `src/privacy/detect/dict.rs`, shown to fail with every name typed
  person.
- Links and addresses in text (Phase 2, [#70](https://github.com/matt-w-horn/protonctl/issues/70);
  built 2026-10-07): a link with any scheme, `webcal://` included,
  becomes one `link N` with its path and query (Q19), and so does a
  domain followed by a path or query, with any top-level domain, or by a
  fragment, with one on the short list. A domain after `@` stays its
  address's, and a file's anchor such as `04-design.md#settings` and
  text such as `and/or` and `km/h` stay as they are. An address in
  letters and digits of any script is found whole, with a mark after a
  letter in decomposed form, and the same link with its host in another
  case keeps its number. A closing quote after a link stays out of it.
  Built: `links_of_any_scheme_or_none_are_found` and
  `addresses_in_any_script_are_found` in
  `src/privacy/detect/pattern.rs`, `other_forms` in
  `src/privacy/canon.rs`, and
  `links_without_an_http_scheme_and_non_ascii_addresses_are_rewritten`
  in `src/privacy/pipeline.rs`, each shown to fail on the patterns of
  2026-10-06: the `webcal://` share link, `www.example.com/reset?token=…`
  and `zoom.us/j/…?pwd=…` kept their path and query, and
  `jürgen.müller@bücher.de` and `张伟@例子.中国` passed raw. The
  exceptions for `@` and for a file's anchor were each shown to fail
  with their rule removed; `ordinary_text_is_left_alone` holds the
  anchor and `and/or`. Not covered: a top-level domain in punycode
  (`xn--…`). A path whose first part looks like a domain, such as
  `Node.js/Deno`, becomes a link, which hides more than it must.
- Evaluation (Phase 2, defect D1 in [section 9](09-rollout.md#phase-2-defects-found-on-real-results)):
  aliases mode over a labeled synthetic corpus that plants each person,
  organization and project in every form D2 to D7 name (surname, given
  name, initials, middle initial, typing and OCR misspellings, run-in
  CJK text, organizations that send mail and that never do, projects),
  and links and addresses that appear only in a body, with and without
  an `http` scheme and in ASCII and other scripts (#70),
  reporting per form and entity type how many came back whole, with a
  word of the name left, with a second alias, linked by `maybeSameAs`, or
  typed wrong. A link's words end at `/`, `?`, `#`, `&` and `=`, and a
  link is typed right when it reads `link N`. The report is a snapshot,
  so each fix shows as the change in its row. Built:
  `what_passes_raw_per_form` in `src/privacy/eval.rs`, shown to fail
  when the process dictionary was emptied (the process-only full name
  came back raw). On the patterns of 2026-10-06 the links without an
  `http` scheme came back 3 of 3 with a word left and typed wrong, and
  the non-ASCII addresses 2 of 2 whole; with #70 fixed, every link and
  address row reads 0. Since 2026-10-08 (M5.2) the corpus is
  `tests/fixtures/names.jsonl`, 146 texts and 315 marked mentions in 28
  languages, with the 28 inline cases moved there, and the report has
  two snapshots: `what_passes_raw_per_form` without the model (recall
  0.137: people 0.185, organizations 0.077, nothing else), and
  `what_passes_raw_with_the_model`, M5.2's exit, where the model is
  installed. The measure finds what each alias replaced to the byte: the
  text between two aliases is the input's unchanged, so an alias starts
  where that text ends, and ends where the input from there has its
  `ref`'s canonical value and is followed by the next text (`eval::replacements`).
  The word alignment it replaced counted a name with a suffix joined to
  it, as Korean and Turkish write them ("김민준님"), as a spurious alias
  and not as a mention, and a link's domain alias as spurious. Built:
  `the_alignment_finds_what_was_replaced`, shown to fail with the `ref`
  ignored (two names with one space between them, "Murat Demir Anadolu
  Lojistik", came back as "Murat" and the rest).
- Recall (Phase 5): per entity type and language, on a labeled synthetic
  corpus in English, German, French and one non-Latin script at least.
  Results are recorded, with no pass mark until there is a measured
  baseline. The same measure decides whether another model may replace
  the one configured (its recall must not fall) and, with the evaluation
  above, whether a dictionary name rule may go (Q23). Built 2026-10-08
  as the snapshot `what_passes_raw_with_the_model` (M5.2's exit), at a
  threshold of 0.15, chosen by `threshold_sweep` (ignored; run by hand
  in a release build): recall 0.943 at 0.1 with precision 0.937, 0.930
  at 0.15 with 0.950, 0.911 at 0.2 with 0.949, 0.848 at 0.3 with 0.952.
  At 0.15, per type (mentions, recall, precision): person 168, 0.935,
  1.000; organization 65, 0.954, 1.000; project 39, 0.949, 1.000;
  product 14, 0.857, 0.444; location 22, 0.818, 0.962; email 3 and link
  4, both 1.000; all 315, 0.930, 0.950. Per language, recall: Arabic,
  Greek, Spanish, Persian, Hindi, Hungarian, Indonesian, Italian, Korean,
  Dutch, Polish, Portuguese, Romanian, Swedish, Swahili, Thai, Turkish,
  Vietnamese and Chinese 1.000 (2 to 13 mentions each); German 0.947
  (19); French 0.929 (14); English 0.906 (138); Japanese 0.900 (10);
  Russian 0.800 (5); Finnish and Ukrainian 0.750 (4 each); Czech 0.667
  (3); Hebrew 0.500 (4). Precision per language is 1.000 except German
  0.905, English 0.913 and French 0.929, where the product label's
  false positives fall. The 16 spurious aliases are 15 products ("smoke
  detectors", "caulk clear", "drill", "Garden Furniture", "Rauchmelder")
  and one location ("marché du samedi"). Per form: both organizations
  that never sent mail are found (D7), 37 of 39 projects (D5), with 1
  of 2 `in-text` projects raw and 6 of 37 full ones typed product or
  organization; the 2 given names, 1 surname, 1 lower-case name, 1
  possessive and 1 honorific form still raw are names the dictionary does
  not know and the model did not take. The dictionary's `split` and `linked` columns are
  unchanged by the model, which links no form to a full name. The proof
  of concept's Python scripts (`scripts/otter-poc.py`,
  `scripts/otter-ocr-poc.py`) were retired with this; the OCR corpus
  `tests/fixtures/ocr-docs.jsonl` stays for Phase 4.
- Names in free text (Phase 5, M5.2; built 2026-10-08,
  [#60](https://github.com/matt-w-horn/protonctl/issues/60)), in
  `src/privacy/detect/model.rs` unless said otherwise. The tests that
  need the model load it once from
  `~/Library/Application Support/protonctl/model` (`model::tests::shared`),
  and where that folder is absent, as in CI on `ubuntu-latest`, each
  prints a notice and checks nothing, so they pass there by design; a
  folder that is there but does not load is a failure. The fixtures
  `tests/fixtures/otter-case.json` (one input with PyTorch's logits) and
  `otter-tokens.json` (Python's token IDs and offsets for 57 texts) were
  written by `scripts/otter-export.py --cases`. Each test was shown to
  fail with the old behavior planted, and the file restored exactly:
  - `special_tokens_are_escaped_to_the_same_length` and
    `names_around_each_special_token_are_found`: each special token of the
    tokenizer, and `[SEP]`, is replaced in text by as many `*`, so
    offsets hold and the prompt gains nothing; names beside each one are
    still found. Shown to fail with the tokens removed instead of
    replaced (the offsets moved), and the second with no escaping at all
    (the window then held more `[LABEL]` tokens than labels, and the
    call failed).
  - `a_missing_or_altered_file_refuses_to_load`: a missing file names the
    folder, and a file with other bytes names the file and its pinned
    SHA-256 (shown to fail with the hash check skipped).
  - `the_runtime_gives_pytorch_s_logits`: `tract` gives the exported
    case's logits within 1e-3 and the same decisions at the threshold.
    `the_tokenizer_gives_python_s_ids_and_offsets`: the `tokenizers`
    crate gives Python's IDs and offsets for every text. With the span
    lengths planted off by one, the self-check refused the model at load
    (`SelfTest`: "Acme" was not found), so every model test failed at
    `shared()`; the logits test cannot be shown failing on its own,
    since any plant in `score` fails the self-check first. The tokenizer
    test compares against the Python fixture and was not planted.
  - `spans_are_listed_as_otter_lists_them`,
    `decoding_keeps_the_best_non_overlapping_spans`,
    `token_spans_map_to_trimmed_bytes` and `the_prompt_is_otter_s`: the
    candidate spans, the decoding and the byte mapping as Otter's
    `predict()` does them (shown to fail with spans ending before the
    last token, with the overlap check dropped, with no trimming, and
    with the prompt's trailing space dropped). The no-trimming plant
    also made the self-check refuse the model, since " Kenji Watanabe"
    with its space is not the name.
  - `windows_overlap_and_their_cores_tile_the_text` and
    `a_name_on_a_window_cut_is_found_once_with_its_offsets`: a text over
    256 tokens is cut into windows overlapping by 64, each reporting the
    core the next does not, so a name on a cut is found once, at the
    text's own offsets (shown to fail with each core starting at its
    window's start: the names in the overlap came back twice).
  - `inference_stops_at_the_deadline`: a deadline already passed stops
    the model before its first window with `Halt::Deadline`, and a
    deadline in the future lets it finish (shown to fail with the
    deadline ignored).
  - `aliases_mode_refuses_without_a_working_model` in
    `src/privacy/mod.rs` (R13): with its key but a model that does not
    load, aliases mode refuses every call with `name_model_unavailable`,
    keeps refusing, and `doctor` names the folder; off mode needs no
    model (shown to fail with a failed load ignored, and with `doctor`'s
    line missing the folder).
  - `names_in_no_header_are_found_by_the_model` in
    `src/privacy/pipeline.rs` (D5, D7): a person, an organization and a
    project in a subject and a body, none in a header, come back as
    aliases, `detectors` names all three detectors, and the organization
    in both texts has one alias in both; the project, read as "Falcon
    rewrite" in one and "Falcon" in the other, gets two, a limit on
    record (Q19). `names_in_free_text_are_aliased_through_the_server` in
    `src/serve.rs`: the same through `Server::call`, which hands the
    session's model to the pipeline. Shown to fail with the model's
    mentions dropped in `detect::add_model`, and the second also with
    the session's model not handed on. `a_search_result_keeps_no_name_id_or_number`
    now expects `regex` and `dictionary` alone, since the pipeline tests
    run without a model (shown to fail with `detectors` always naming
    the model). The leak test over every tool (`src/serve_every_tool.rs`)
    runs without the model too, so its two known gaps stand as before.
  - `measure` (ignored; release build): time per page and the process's
    RSS with the model loaded. On 2026-10-07 on an M2 Pro with 16 GB, a
    20,040-character page of 5,522 tokens took 7.23 s on four threads in
    windows of 256 (10.3 s in windows of 896, 9.0 s in 512), a 44-token
    sentence 41 ms, and the test process held 1,426,464 KB.
- `scripts/live-check.py` (Phase 2; real data, counts only): every result
  with aliases carries `entities`, and every entry a `ref`; no text field
  outside `entities` matches an email-address, phone-number or 40- or
  64-hex-digit pattern; no `nodeId`, Proton ID or image block appears;
  every tokenized result names its `detectors`; a second server run gives
  the same aliases for the same search; the `reveal_*` tools run only with
  `--reveal`, which needs Touch ID at the Mac. In off mode the checks of
  2026-10-03 stay, among them the export folder, `download_file`, raw
  digests and 16-character messageIds. Built in part: in aliases mode the
  script counts addresses, links, local paths and 40- or 64-hex-digit
  digests in every result, and phone numbers (international with a `+`,
  North American, or national with a leading 0) in every string outside
  `entities`; it fails on a missing `detectors`, an entity without a
  `ref`, a `dropped` field or content after the JSON, and searches again
  by a sender's `ref`. It then starts a second server and repeats one
  mail search that leaves out today's mail, failing on any alias that is
  new, gone or of another type (2026-10-06,
  [#14](https://github.com/matt-w-horn/protonctl/issues/14); not yet
  run live). In both modes a failed call prints only its error code, or
  `error text` when the error has none, and the message's length, never
  the message, which in off mode can carry a Drive path or the Drive
  CLI's stderr (2026-10-07,
  [#75](https://github.com/matt-w-horn/protonctl/issues/75)). `--reveal`
  comes with Phase 3.

R1 is tested for both services that reach the account. The stand-in
`proton-drive` in `src/drive/mod.rs` and `tests/convert.rs` answers only
the four calls protonctl makes, `filesystem list`, `filesystem info`,
`filesystem download` and `--version`, in their exact shapes; any other
call fails and is recorded, and the test fails on the record when it
ends, even when the code went on past the failure. For Mail, R1 rests on
the commands sent: mailboxes opened with EXAMINE, never SELECT, and bodies
fetched with `BODY.PEEK`, which sets no `\Seen` flag. The scripted
Bridge's test ends by checking every command it received against LOGIN,
LIST, EXAMINE, STATUS, UID SEARCH, UID FETCH and LOGOUT, with no fetch of
`BODY[` or an `RFC822` item; `commands_that_can_change_the_account_are_refused`
in `src/mail/read.rs` pins that the check refuses the others.

The scripted Bridge's operations also run against a real IMAP server (T11, built
2026-10-06, [#10](https://github.com/matt-w-horn/protonctl/issues/10)):
`scripts/with-dovecot.sh` starts Dovecot 2.3.21, pinned by digest, in a
throwaway podman container with `tests/fixtures/dovecot.conf`, which
gives Bridge's mailbox names and roles over implicit TLS. The ignored
test `every_operation_against_dovecot` seeds the scripted Bridge's
messages with APPEND from a session of its own, reads them through a
`Mail`, and holds every result to the scripted Bridge's snapshots;
`list_labels` is compared in Bridge's mailbox order, since IMAP leaves the
order of LIST to the server. It then reads every mailbox and every
message's flags but `\Recent` again, and fails if any changed. That check
failed when a read used both SELECT and `BODY[]`; with `BODY[]` alone
nothing changed, since a mailbox opened with EXAMINE is read-only. So it
catches a change only where a read opens a mailbox with SELECT; which
commands are sent stays the scripted Bridge's check, over the same
operations. `scripts/check.sh` runs it on Linux where
podman is installed.

Not built: an optional prompt-injection drill (about 5 Claude runs), extended
to check that injected text gets no raw content without Touch ID, and
sends nothing out through the host's other tools ([#11](https://github.com/matt-w-horn/protonctl/issues/11)).
The sandbox-only live write tests are withdrawn with the writes.

---

[← 6. Privacy](06-privacy.md) · [Contents](../rfc-0001.md#contents) · [8. Review process →](08-review-process.md)
