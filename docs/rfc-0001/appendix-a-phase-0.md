[RFC-0001](../rfc-0001.md) › Appendix A: Phase 0 findings

# Appendix A: Phase 0 findings

Recorded 2026-10-03 against Proton Mail Bridge 3.27 in SSL mode, with a
temporary probe that returned metadata only: header names, short `X-Pm-*`
values, the length and character class of longer values, capabilities,
mailbox roles, body encodings and timings. No message content was read.
Entries marked *live check* come from the built tools, run against the same
mailbox by a script that printed only counts, booleans and timings.

**Mail**

- Capabilities: IMAP4rev1, AUTH=PLAIN, UNSELECT, IDLE, STARTTLS, UIDPLUS, ID,
  MOVE. No SORT, THREAD, CONDSTORE or ESEARCH, so protonctl computes date
  order and thread grouping itself. UIDPLUS and MOVE make draft replacement
  and moves safe for Phase 1b (withdrawn 2026-10-04).
- Mailboxes carry special-use attributes: `\All` (All Mail), `\Archive`,
  `\Drafts`, `\Flagged` (Starred), `\Junk` (Spam), `\Sent`, `\Trash`, plus
  INBOX. Labels appear under `Labels/`; folders under `Folders/`, a
  `\NoSelect` parent. The `Labels` parent is listed without `\NoSelect`, but
  STATUS on it fails with "no such mailbox", so `list_labels` skips any
  mailbox STATUS refuses (*live check*). async-imap does not decode modified
  UTF-7 mailbox names; none in the test mailbox uses it, so a non-ASCII label
  name would appear encoded.
- Every sampled message carries `X-Pm-Internal-Id` (88 base64-like characters:
  Proton's message ID), `X-Pm-Gluon-Id`, `X-Pm-Date`, `X-Pm-Origin`
  (internal, external, import), `X-Pm-Content-Encryption` (end-to-end,
  on-delivery, on-compose) and `X-Pm-Spamscore`; most also carry
  `X-Pm-External-Id` and `Authentication-Results`. There is no
  conversation-ID header.
- Every sampled message's `References` includes an ID at
  `protonmail.internalid`. It is the message's own ID, not a conversation
  key: as a thread key it equalled `X-Pm-Internal-Id` for all 20 messages of
  a `newer_than:30d` search (*live check*). So a thread is named by its root
  message, the first other ID in `References`, else `In-Reply-To`, else the
  message's own `Message-Id`, and `get_thread` finds the root plus every
  message whose `References` names it. *Live check:* five distinct threads (2
  to 6 messages, one root ID starting with `-`) each held the message
  searched from, and every message in each carried that thread's `threadId`.
- `X-Attached` appears once per attachment. A HEADER search with an empty
  string matches every message, with or without the header:
  `HEADER X-Attached ""` returned all of the 20 newest messages, with
  attachments or without, and its negation none (*live check*). So
  `has:attachment` never reaches SEARCH: each fetched summary's `X-Attached` count decides,
  and after that change the same searches returned 20 of 20 with attachments
  and 20 of 20 without. Non-empty HEADER strings match as substrings.
- Text bodies arrive quoted-printable (HTML in nearly all sampled messages);
  calendar parts arrive base64.
- Timings on a test mailbox of about 20,000 messages in All Mail:
  `UID SEARCH TEXT "the"` 1.5 s; `UID SEARCH HEADER X-Pm-Internal-Id <id>`
  2.3 s, one hit with the right UID; `UID FETCH 1:* (INTERNALDATE)` 0.8 s.
  UID order runs backwards against date at about 4% of steps, so results are
  sorted by INTERNALDATE, and the ID-to-UID mapping from each search is kept
  in memory so opening a found message skips the 2.3 s lookup.
- Cold start (Q2), 2026-10-03: with the Bridge app quit, its core
  (`bridge --noninteractive`) listened on 1143 after 0.9 s and accepted
  protonctl's login 10.7 s after launch.
- Second probe, 2026-10-03 (counts and timings only), on the same mailbox:
  - Every `INTERNALDATE` is UTC (`+0000`), so `SINCE` and `BEFORE` compare
    UTC dates.
  - Fetching all of them in chunks of 2,000 UIDs: `INTERNALDATE` 2.4 s,
    `ENVELOPE` 1.9 s, and the header fields
    `FROM TO MESSAGE-ID X-PM-INTERNAL-ID X-ATTACHED` 4.7 s.
  - Every `X-Pm-Internal-Id` is 88 characters, and their first 16
    characters are unique, also compared case-insensitively, as IMAP's
    HEADER search compares them.
  - `Message-Id` is not unique: about a fifth of the distinct values appear
    on 2 to 4 messages. In nearly all of those groups every copy carries
    `X-Pm-Origin: import`: an import can store a message more than once. A
    few groups differ in From or Subject.
  - `BODY.PEEK[TEXT]<0.32768>` returned data for the 20 newest messages (up
    to 32,768 bytes each, 0.01 s). Joined to their `CONTENT-TYPE` and
    `CONTENT-TRANSFER-ENCODING` header fields, 17 of the 20 parsed to
    non-empty text.
- Bridge copies a message's own headers, then sets `X-Pm-Internal-Id`,
  `Message-Id` and `References` with go-message's `Set`, which replaces any
  copy the sender wrote (`pkg/message/build.go`, `setHeaderIfNeeded`). Other
  headers keep the sender's order, so a sender's own
  `Authentication-Results` can sit below the receiving server's. mail-parser's
  `header_raw` returns a header's last copy.

**Drive CLI** (`cli-drive@0.8.0`), recorded 2026-10-03 by a probe that printed
keys, types and sizes only:

- Paths are POSIX: `/my-files/...` for the account's own files,
  `/shared-with-me/NODE-UID/...` for shared ones, and a `/` inside a name is
  escaped with a backslash. A path in the Proton Drive app's folder maps to
  `/my-files` plus the same path (*live check*: three files fetched this
  way). `realpath(3)` returns names as stored, case and Unicode normalization
  included, so the resolved path carries the exact names. Whether the CLI
  wants a backslash inside a name escaped is not tested
  ([#32](https://github.com/matt-w-horn/protonctl/issues/32)).
- `-j` must follow the subcommand (`filesystem list -j PATH`); before it, the
  CLI prints usage and exits 1.
- `list` prints an array of nodes and `info` one node: `uid`, `parentUid`,
  `name` and the key and name authors as `{ok, value}` results (a name can
  fail to decrypt or verify), `type` (`file`, `folder`), `mediaType`,
  `isShared`, `isSharedByUrl`, times, and for a file an `activeRevision` with
  `claimedSize`, `storageSize`, `contentAuthor` and a claimed SHA-1
  (`sha1Verified: false`). `totalStorageSize` reads 0 for files.
- `download` prints `{transferredItems, transferredBytes, skippedItems,
  failedItems, failures}` and prompts on a name conflict unless given `-f`
  and `-d` strategies; protonctl passes `skip`, an empty stdin and a new
  folder per download.
- `download` writes each file straight to its place in the folder it is
  given, through a Bun file writer that the SDK streams decrypted blocks
  into, and deletes it if the download fails; neither the CLI nor the SDK
  writes a copy anywhere else, the system's temporary folder included. So
  in aliases mode a file read through the CLI touches only the memory
  folder (R10). The CLI also keeps an encrypted SQLite cache of keys and
  node metadata in its own cache folder (`proton-drive-cli` under
  `~/Library/Caches` or `$XDG_CACHE_HOME`). Read in the CLI's and the SDK's
  source at commit 28ac9cd on 2026-10-05 (`downloadOperations.ts`,
  `fileDownloader.ts`, `cache/index.ts`).
- The CLI reads `PROTON_DRIVE_BASE_URL` (its API host),
  `PROTON_DRIVE_CREDENTIALS_STORE` (`keychain`, `unsafe_file` or `pass`),
  `PROTON_DRIVE_UNSAFE_CACHE`, `PROTON_DRIVE_CACHE_DIR` and
  `PROTON_DRIVE_LOG_LEVEL` (`DEBUG` unless set; `INFO`, `WARNING`, `ERROR`),
  and reaches the Keychain through the Security framework itself. protonctl
  clears the environment and sets only `HOME` and
  `PROTON_DRIVE_LOG_LEVEL=ERROR`.
- Timings: `list` of the top folder 4.3 s, `info` 4.6 s, `download` of a
  small file 3.2 s. *Live check:* `drive get` saved a cloud-only file and a
  local file at their listed sizes (5.3 s and 7.8 s); the cloud-only file
  stayed cloud-only, so the Drive app fetched nothing; `drive cat` read a
  cloud-only text file through the CLI; no download folder was left behind.
- Errors: a missing path exits 1 with `Node not found: NAME` on stderr,
  naming the last part (protonctl turns this into the `not found` an
  excluded path gives, R7); `list` of a file exits 1 with `Invalid link
  type. Expected: folder, found: file`, after a stray `[` on stdout.
  `/my-files/` is accepted like `/my-files`. Folders carry no
  `activeRevision`, and `modificationTime` is RFC 3339 with milliseconds.
- From the CLI's bundled source (read 2026-10-03): a download is saved under
  the node's name with control characters and `<>:"|?*\/` replaced by `_`
  (an empty name or `.` becomes `_`, `..` becomes `__`), so protonctl takes
  the one file in a new folder rather than predicting the name. A path part
  shaped like a node UID (two base64url halves joined by `~`) is looked up
  by UID wherever the node lives, and `\/` in a path is a `/` inside a name;
  protonctl refuses both shapes, since the first would pass exclusions by
  name (R7) and the second, after a folder name ending in `\`, would reach
  another node. `--version` also fetches
  `https://proton.me/download/drive/cli/version.json` (5 s limit) to report
  a newer release; protonctl runs it once per process, with the same cleared
  environment as every other call. *Live check:* `drive get` through a new
  folder saved a file at its listed size and left only that file.
  *Live check* (protonctl without the app's folder, counts only): the top
  folder listed with no entry skipped, and `get_file_metadata` of a file
  matched its listing entry.

<a id="memory-disk-m28"></a>**Memory disk** (M2.8, recorded 2026-10-04 on macOS 27, by hand and through
`platform::memory_disk`): `diskutil image attach --noMount ram://<sectors>`,
`newfs_hfs` and `diskutil mount -mountOptions nobrowse,owners,noexec
-mountPoint <folder>` all run as the user, with no admin password;
`hdiutil attach` still works but warns that it is deprecated. The mount
reads `hfs, local, nodev, noexec, nosuid, nobrowse, mounted by matt`, and
`diskutil info` reports `Owners: Enabled`, so the volume's 0700 root holds.
The device nodes come up `brw-r----- matt staff`, readable by group
`staff`, which every local user is in, so another account could read the
raw blocks around the volume's permissions; protonctl makes both nodes 0600
before formatting. A canary file stayed out of Spotlight for 90 s, while
`mdfind` found a file on the system volume at once; the volume gains no
`.Spotlight-V100` folder, and `nobrowse` keeps it out of Finder.
`vm.swapusage` reports the swap as `(encrypted)`.
`diskutil eject` removes the disk and its device nodes. *Live check*
(aliases mode, counts only): a cloud-only Markdown file of 7,275 bytes
came back as 7,907 characters of aliased text; it stayed cloud-only, and
no download folder, memory folder or mounted disk was left after the
server exited. A folder made online-only in the Drive app is itself
dataless, and its listing is not on the Mac: `read_dir` gave nothing, so
`list_folder` and `list_drive_tree` showed it as empty and its 126 files
not at all, with no note. After a second change in the app, 126 files
and one folder were dataless, and the tree reported 40 cloud-only files
before the check stopped at the first one it could read.

<a id="signing-q12"></a>**Signing** (Q12, recorded 2026-10-04 on macOS 27, in a throwaway keychain,
with a probe that reads a generic password with user interaction turned
off, so a refusal returns an error instead of a dialog): `codesign` signs
with a self-signed identity that `find-identity` marks
`CSSMERR_TP_NOT_TRUSTED`, so no trust setting has to change. The
designated requirement is `identifier "probe" and certificate leaf =
H"..."`. An item created for the signed probe was read with status 0 by a
rebuilt probe signed with the same identity, and refused (`-25293`) by an
ad-hoc build and by one signed with a second self-signed identity. A key
imported with `-T /usr/bin/codesign` signed with no prompt; imported
without `-T`, with or without `-x`, `codesign` waited for approval, but
the key's ACL then trusts `/usr/bin/security` for `sign` and
`export_clear`; `-T ""` leaves the `sign` entry trusting no program
(read with `security dump-keychain -a`, which raises no dialog). On the
login keychain, choosing Always Allow for `codesign` on the first install
put it in that entry, and every later install signed silently until it
was removed in Keychain Access. An
unsigned build's identifier is `protonctl-<hash>`, new each build, so the
install passes `--identifier protonctl`. macOS's own LibreSSL 3.3.6 makes
the certificate and the PKCS#12 file `security import` reads.

**Calendar** (recorded 2026-10-03, headers only; the link went from the
Keychain to curl's stdin): the feed is served with `Cache-Control: max-age=0,
must-revalidate, no-cache, no-store, private`, an `Expires` in 1984, and no
`ETag` or `Last-Modified`, so no conditional request is possible and every
fetch is a full download. protonctl's 15-minute memory cache (R12) is the
only caching; the lag of up to 8 hours is Proton's, before the feed is
served. The feed is not limited to recent events (checked month by month on
2026-10-03), so for older events the `search_events` default window (from 90
days back) limits results, not the feed, and a search for an older range that
sets no window finds nothing. An empty result over a defaulted window
therefore carries a `note` naming the window and saying to pass `startTime`
and `endTime`.

---

[← 11. Platforms](11-platforms.md) · [Contents](../rfc-0001.md#contents) · [Appendix B: Proton's open-source code →](appendix-b-proton-code.md)
