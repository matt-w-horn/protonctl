[RFC-0001](../rfc-0001.md) › 7. Testing

# 7. Testing

Built (on macOS, `cargo test` lists 191 tests: 176 unit tests and 15
against the built binary, of which 4 are ignored by default: one lists
Drive through the real CLI, one checks the real CLI's Developer ID
signature, one reaches Dovecot, and one runs as the child of another
test. The Linux counts were last taken before aliases mode was removed,
and are not given here. The macOS-only tests do not run on Linux, and the
Linux-only tests do not run on a Mac). Live,
`scripts/live-check.py` runs every tool once over MCP, as the Claude app
does, against the real calendar link, Bridge and Drive, and prints counts
only. First, all 15 tools then built passed; its provenance check
caught a `read_file_content` reply for a file too large to return as text
that carried no `provenance` (R6), now fixed and covered below. Later
all 17 passed again, with the checks added since: 16-character
messageIds and no `encryption` in search rows, snippets, digests on saved
files, `count_messages`, an export into the export folder, and a complete
manifest with its SHA-256, each removed afterwards. Then it
passed in aliases mode on a Mac, all 16 tools of that mode, and later
in off mode, all 18 tools (M1.2 in [section 9](09-rollout.md)).
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
- The Drive CLI's check (R9, Q24, Q33), against stand-in CLIs. Every
  run is of the private copy, on Linux a sealed memfd run as
  `/proc/self/fd/N`. A `#!/bin/sh` script cannot run from there,
  since the fd is close-on-exec and the shell cannot open that path, so
  each stand-in is one `#!/usr/bin/env -S /bin/sh` line that runs its
  script, and the trusted stand-ins of the Drive tests run through the
  copy too.
  - A pinned CLI runs only while it matches its pin, checked before every
    run, with `--version` once per process; one with no pin, or a pin for
    other bytes, never runs.
  - A CLI swapped in after the check never runs. The stand-in renames
    another CLI over itself while it answers `--version`, between the
    check and the command, as the security review's stand-in did. The
    command's own copy is made and checked after `--version`, so it is of
    the swapped file and that call is refused before anything runs, as is
    the next; on Linux every run is of `/proc/self/fd/N` (shown
    to fail with the command run by path, as before the fix, and with only
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
    macOS. `setup drive --cli` on a set-up Drive changes only `cli` and
    removes a `cli_sha256` left from before, comments and every other
    value kept. The copy hashes each byte it writes, across chunks.
  - Not automated: the whole flow on a pseudo-terminal from `script`, in a
    throwaway keyring: a first setup confirmed with `yes`
    stored the pin and wrote only `cli`; `status` and `doctor` showed the
    pin; a piped `setup drive --cli` was refused, and on a terminal it
    pinned the new CLI and removed an old `cli_sha256`. The CLI runs on
    Bun ([Appendix A](appendix-a-phase-0.md)), likely as a Bun standalone
    binary, which reads its own code from its executable: one built with
    Bun 1.4.2 ran from a sealed memfd and found its code. The real CLI, on
    a Linux machine signed in to Proton, has not run this way.
- Surface: no forbidden tool names; every hint set and every description
  within 2,048 characters; one snapshot of the whole tool surface, names and
  descriptions included, since Cowork re-approves a tool whose definition
  changes; misspelled parameters refused (the test failed first:
  `start_time` deserialized to an empty window). The snapshot holds
  `readOnlyHint: false` on the three tools that can leave a file behind
  (`get_attachment`, `download_file`, `export_drive_manifest`; M1.6) and
  true on the rest. The forbidden names do not list the withdrawn write
  tools; the snapshot's exact set of names keeps them out. Built:
  `tool_surface_snapshot`, `no_tool_offers_a_forbidden_capability`,
  `the_forbidden_name_check_fires_on_a_bad_list`,
  `every_tool_sets_every_hint_explicitly` and
  `misspelled_parameters_are_refused` in `src/serve.rs`.
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

- Found by the [security assessment](../security-assessment.md), each
  shown to fail on the code before its fix:
  - The export folder (R10): with a symlink at a part of the export folder,
    deeper in it, or at the export folder itself after it was checked, the
    refusal leaves the place it leads empty (the old code made a folder
    inside the Drive app's folder). A download is staged where only
    protonctl can write, and one the stand-in leaves as a symlink is
    refused (the old code staged inside the export folder and returned the
    digest of the file the symlink named). A later manifest page swapped
    for a hard link, then a symlink, is refused and the target is unchanged
    (the old code appended through the hard link). An exported attachment
    whose name holds a hard link to an outside file with the same bytes is
    refused (the old code answered as an equality test), and a named pipe
    there is refused without hanging. Built:
    `the_folder_must_be_absolute_and_outside_drive_and_the_cache` and
    `a_symlink_anywhere_on_the_way_is_refused_with_nothing_made_outside` in
    `src/export.rs`, `an_export_is_staged_where_only_protonctl_can_write`
    and `a_manifest_page_refuses_a_link_swapped_in_for_its_file` in
    `src/drive/mod.rs`, `an_exported_attachment_is_saved_once` in
    `src/mail/read.rs`.
  - Bounded work (R8): 1 MiB of `<` and 256 Ki `<div` in an HTML body each
    finish in under 1 s, and a quote after a `<` in text is still stripped
    (the old scan was still running after 10 s). 200 daily series over a
    year give `complete: false` and a note, and 2 give `complete: true`
    (the old code expanded all 73,200 occurrences). A thread is fetched a
    bounded size at a time and comes out the same; an attachment over
    64 MiB is saved, not read. A `--version` that never ends is killed and
    the CLI's lock is free again (the old code waited past 30 s). Built:
    `many_unclosed_tags_are_read_about_once` in `src/mail/body.rs`,
    `a_query_stops_at_the_cap_across_the_feed_and_says_so` in
    `src/calendar/mod.rs`, `expansion_stops_when_the_budget_runs_out` in
    `src/calendar/ics.rs`, `a_thread_is_fetched_a_bounded_size_at_a_time`
    and `an_attachment_over_64_mib_is_saved_not_read` in
    `src/mail/read.rs`, `a_version_check_that_hangs_is_killed_and_frees_the_lock`
    in `src/drive/cli.rs`. The R1 guard on Bridge commands allows
    `RFC822.SIZE` and still refuses every other `RFC822` item.
  - The Drive CLI (R9): a stand-in that overwrites the copy it runs from
    while it answers `--version` never gets its new bytes run (the old code
    ran them). By hand, outside Claude Code's sandbox: the installed CLI
    meets the Developer ID Application requirement and `/bin/ls` does not
    (it passed with a wrong field planted). Built:
    `the_command_runs_a_copy_checked_just_before_it` and
    `only_proton_s_developer_id_signature_passes` in `src/drive/cli.rs`.
  - Hidden characters (R6): emoji, ideographic variation sequences,
    keycaps, ZWJ sequences and jamo sequences stay; runs of selectors,
    selectors with no base and stray fillers go and are counted (the old
    code kept `a\u{E0141}\u{E0142}\u{E0143}b` whole). A calendar's name
    from the feed or the config loses U+202E and tag characters, and a
    label name shows U+202E as `\u{202E}` and is found by it. Built:
    `selectors_and_fillers_stay_only_where_text_is_written_with_them` in
    `src/content.rs`, `calendar_names_are_cleaned` in
    `src/calendar/mod.rs`, `hidden_characters_in_label_names_show_as_escapes`
    in `src/mail/read.rs`.
  - Exclusions and names (R7): a symlink to a folder only in the cloud is
    listed through the CLI where it leads, with the excluded child left
    out (the old code listed the symlink's own path and came back empty).
    `Report.txt` keeps its own node and `report.txt` is listed apart (the
    old code gave the first the second's node). Built:
    `a_symlink_to_a_folder_only_in_the_cloud_lists_where_it_leads` and
    `names_differing_only_in_case_keep_their_own_nodes` in
    `src/drive/mod.rs`.
  - The calendar link (R3): `[1-100]`, `{a,b}`, `]` and `}` are refused,
    and curl gets `--globoff` (the old check took `x[1-100].ics`). Built:
    `only_https_proton_links_pass` in `src/calendar/mod.rs`.
  - Readers (R21): a one-page PDF whose text holds form feeds reads as one
    page (the old script gave three); `doctor`'s sample run of a reader
    that never ends stops at its limit and is killed. Built:
    `form_feeds_in_a_page_s_text_do_not_split_it` in `tests/convert.rs`,
    with `tests/fixtures/form-feeds.pdf`, and
    `a_sample_run_that_never_ends_stops_at_its_limit` in `src/convert.rs`.
  - Logs (R25): an unknown tool, wrong and unknown arguments,
    `resources/read`, `prompts/get` and an unknown method each carry a
    marker that never reaches stderr (the old code's library logged the
    method name); a server that cannot start names no path or address.
    Built: `refused_requests_stay_out_of_stderr` and
    `a_server_that_cannot_start_names_no_path_or_address` in
    `tests/serve_exit.rs`.
  - Proton Docs and Sheets: reading, downloading (to the export folder
    too) and metadata each name the export from the web app, no CLI
    download runs, the placeholder stays as it is, and rows carry
    `protonDoc` (the old code returned empty text). Built:
    `proton_docs_and_sheets_say_how_to_read_them` in `src/drive/mod.rs`.

Each guard was first shown to fail on a constructed bad input: the
forbidden-name check on a list containing `send_message`; the exclusion test
against the same tree without the exclusion; the snapshot after renaming one
tool title; the HTTP-crate ban after adding `ureq`. The symlink test found a
real gap (exclusions were checked only on the requested path) and failed until
`resolve` also checked the resolved one. The tests added in an early
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

More tests, each shown to fail on a constructed bad input before it is
trusted:

- Logs (R25): with `RUST_LOG=debug`, stderr holds no tool result, and a
  planted panic prints none of its message. Built:
  `results_stay_out_of_stderr_even_with_rust_log_set` in
  `tests/serve_exit.rs` looks for a phrase of `get_status`'s result, and
  `a_panic_prints_where_and_withholds_what` in `src/main.rs` and
  `a_blocking_panic_withholds_its_message` in `src/content.rs` plant a
  panic message.
- Converter sandbox (Phase 4): a test build of the helper that tries to
  open a socket, write a file and read a Keychain item fails at each.
  Built on Linux, ahead of Phase 4: `the_sandbox_confines_a_reader`, which
  runs `sandbox_probe` as a child, in `src/platform/linux.rs`, and
  `each_reader_runs_in_its_sandbox` and
  `a_damaged_pdf_fails_in_the_reader_not_the_sandbox` in
  `tests/convert.rs`. Built on macOS (M4.1, Q13):
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
  exit-code test with `sandbox-exec`'s codes read as a reader's.
  Memory and signals: a 200 KB Word file holding
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
  The x32 numbers:
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
- Secrets wiped on drop (R2): on Linux,
  `secrets_are_wiped_on_drop` in `src/secret.rs` drops a secret, then
  reads the freed block through `/proc/self/mem`, so no unsafe code reads
  it, and expects zeros past the allocator's own 16 bytes. It first
  checks that a dropped plain `String` still shows its text, so an
  allocator that returns the block to the system cannot pass the test.
  With a plain `Box<str>` in place of the secret, it failed.
- `scripts/live-check.py` (real data, counts only): it runs every tool
  once in the one mode, with the checks of its first runs, among them the
  export folder, `download_file`, digests on saved files and
  16-character messageIds. A failed call prints only its error code, or
  `error text` when the error has none, and the message's length, never
  the message, which can carry a Drive path or the Drive CLI's stderr.

The privacy layer's tests and test plans (settings, the pipeline, the leak
test, identifiers, names in text and in free text, the evaluation, recall,
no content on disk, user presence and the role-play) were withdrawn with
aliases mode by [Q43](10-open-questions.md).

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

The scripted Bridge's operations also run against a real IMAP server (T11):
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

Not built: an optional prompt-injection drill (about 5 Claude runs), to
check that injected text sends nothing out through the host's other tools.
The sandbox-only live write tests are withdrawn with the writes.

---

[← 6. Privacy](06-privacy.md) · [Contents](../rfc-0001.md#contents) · [8. Review process →](08-review-process.md)
