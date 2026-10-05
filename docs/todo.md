# To do

Every open item in one place: bugs, documentation that states something
wrong, missing tests, reviews, the checks that need a Mac or a Linux
desktop, decisions, and work not started. Milestone states live in
[RFC section 9](rfc-0001/09-rollout.md); this file lists what is left.
When an item is done, delete it; the commit that does it says so.

Where: **here** can be done in a Linux container; **Mac** and **Linux
desktop** need that machine and, mostly, a person; **decision** needs the
maintainer.

## Bugs

- B1 (here) On Linux, every unit-test run leaves an empty
  `protonctl-<pid>` folder in `/dev/shm/protonctl-test-<uid>` (22 on
  2026-10-05). A server killed before it exits leaves its folder under
  `$XDG_RUNTIME_DIR` the same way, possibly with a file in it, until
  logout.

## Documentation that states something wrong

- D1 (here) `09-rollout.md`: the phase table says P2 to P4 are to do and
  leaves the Linux readers out of Phase 4; M2.10 says the live runs wait
  for a Mac, but the aliases-mode run passed.
- D2 (here) `security-privacy-review.md`, the invariants: I3, I6, I7, I10,
  I11, I12, I13, I16 and I18 read "planned" though they are built; I3 says
  secrets live only in the Keychain.
- D3 (here) `security-privacy-review.md`, the threats: "medium until Phase
  2" for a replaced binary (Q12 is done), "medium until M2.9" for panics
  (done), and the converter row without the Linux sandbox; the closing
  table's lists of built and planned checks.
- D4 (here) `security-privacy-review.md`: the data-flow diagram was not
  checked again when Phase 2 closed, and lacks `protonctl convert`, the
  Secret Service and the memory folder.
- D5 (here) `10-open-questions.md`: Q18's row says the licence is still to
  check; it was checked on the Mac.
- D6 (here) `02-requirements.md`: R2 says secrets are kept in the macOS
  Keychain only.
- D7 (here) `07-testing.md`: the list of planned privacy-layer tests does
  not say which exist; the counts at the top are from 2026-10-04.

## Tests the RFC plans that do not exist

- T1 (here) A leak test over every tool with a planted corpus; today one
  covers mail search, events and the Drive listing.
- T2 (here) A test that calls every tool through the server (I9).
- T3 (here) The fail-closed path: a pipeline panic becomes
  `pipeline_failed`.
- T4 (here) Aliases stable across two processes under one key, and
  different under another.
- T5 (here) Alias collisions, which need a word list small enough to
  collide; the list cannot be swapped in today.
- T6 (here) Key rotation at the server: new aliases, old handles refused.
- T7 (here) No new file anywhere in the home folder after every tool runs
  in aliases mode.
- T8 (here) No image, embedded resource or file bytes from any tool in
  aliases mode (R22).
- T9 (here) Errors and page tokens carry no name, path or UID in any
  encoding.
- T10 (here) The read-only guarantee (R1): a stand-in Drive CLI that fails
  on any call but the four protonctl makes, and a test that refuses any
  IMAP command outside LOGIN, LIST, EXAMINE, STATUS, UID SEARCH, UID FETCH
  and LOGOUT.
- T11 (here, larger) A hermetic IMAP test against Dovecot in podman.
- T12 (later) A prompt-injection drill, about 5 Claude runs.
- T13 (here) A locked Secret Service collection: what a read does when the
  unlock prompt cannot be shown. The tests always run unlocked.
- T14 (not testable here) The x32 seccomp rule, which needs unsafe code or
  a kernel built with x32; and that secrets are wiped on drop (R2), which
  needs a look at freed memory.

## Reviews

- R1 (here) The correctness review of the Linux work (MP2 to MP4) is
  running; its findings are added here.
- R2 (decision) Q25: `/security-review` on the Phase 2 changes has no
  record of a run. The security review of the Linux work was by reading
  only, since probing the sandbox was stopped by the model's safeguards.

## Needs a Mac

Passed on 2026-10-04, with the binary installed from `5c50048`: the gates
(195 unit and 6 binary tests, PDFKit and `textutil` included); `status`,
`doctor` and `get_status` with no mode set; `setup privacy` creating, then
keeping, the key, with both items' comments as designed; a running server
answering `privacy_mode_changed` after the mode was set under it;
`setup privacy --off` and `rotate-key` refusing without a terminal; and
`setup drive`, then its refusal on a second run. `get_status` went over a
direct MCP session with the installed binary, against an empty config, so
no secret was read. The mode is `aliases` now. M2.8 and the word list's
licence are done; a cloud-only file was read live in aliases mode
through the RAM disk. `live-check.py` passed in aliases mode (16 of 16
tools); its first run found page edges cutting addresses in two, fixed in
`d06db50`. Signing (Q12) passed the same day: `scripts/install.sh`
made the identity, and a rebuild signed with it loaded the privacy key
with no prompt. The first install's Always Allow for `codesign` had made
later installs sign silently; it and `/usr/bin/security` were removed
from the key's access list, which now trusts no program.

Each step below needs a person: a Keychain dialog, a terminal prompt, or a
host restart, except M1, which needs only a Mac.

### M1. After pulling the Linux work

The Linux work (MP2 to MP4) changed code that runs on a Mac too, linted
there from Linux but not run.

1. Run `scripts/check.sh`. The PDFKit page rendering moved into
   `pdfkit_pages` in `src/extract.rs`;
   `pdf_pages_come_as_images_and_a_scan_s_without_asking` checks it.
2. Install, then run `protonctl status`. Expect your existing config to
   load: `cert_sha256` in `[mail]` is now read as a SHA-256 when the
   config loads, so a malformed value would stop every command.
3. In off mode, call `download_file` twice in Claude Code; it always runs
   the Drive CLI, whose signature is now checked before every run (Q24),
   one `codesign` call each. Expect both calls to pass, and note how much
   time the check adds per call.

### M2. The Keychain code, through the hosts

1. With a server still running, run `protonctl setup privacy --off` in
   a terminal and confirm. Then call any tool: expect
   `privacy_mode_changed`. Restart Claude Code: expect off mode.
2. Run `protonctl setup privacy`, restart, and note one alias. Run
   `protonctl rotate-key` (it asks), and repeat the call. Expect a
   different alias, with no restart.

### M3. Live checks (M1.1, M1.2, M2.10)

`scripts/live-check.py` runs in either mode; it has passed in aliases mode.

1. In off mode, run `python3 scripts/live-check.py` from a normal
   terminal. Expect "all calls passed".
2. In Claude Code and in Cowork (M1.1, M1.2), ask one question per
   service in each mode.
3. Repeat the role-play of
   [Appendix C](rfc-0001/appendix-c-roleplay.md) on real results in
   aliases mode. Record only its findings.

### M4. Last: logout

`protonctl logout` deletes every protonctl Keychain item, the calendar
links and the Bridge password included, so `setup calendar` and
`setup mail` must run again afterwards. Run it only when ready to redo
them. Expect it to ask, then to list `privacy-key` and `privacy-mode`
among the deleted items. The next call gives `privacy_mode_unset`.

### M5. Open questions that need a Mac

- Q13: the sandbox profile for PDFKit and `textutil` (Phase 4).
- Q15: whether a LocalAuthentication prompt appears under each host and
  inside Claude Code's sandbox (Phase 3).
- Q17: the Claude Code sandbox settings (Phase 3).

## Needs a Linux desktop

- L1 MP2's live check: mail and calendar, with Bridge as a systemd user
  unit.
- L2 MP3's live check: Drive through the real CLI, signed in.
- L3 Whether Claude Desktop's Linux beta loads local MCP servers, and how
  Cowork's VM reaches them (section 11).
- L4 What a call does when the keyring locks while a server runs, since
  every call reads the privacy setting.
- L5 Bridge's install path, which the README's unit file names.
- L6 (any system) Whether the Drive CLI wants a backslash inside a name
  escaped (Appendix A).

## Decisions

- Q13, Q15 and Q17 (Mac), and Q23 (Phase 5): see
  [section 10](rfc-0001/10-open-questions.md).
- Phase 3: whether `reveal_*` also carries Claude Code's own confirmation
  (`04-design.md`), and whether a reveal may return page images
  (`06-privacy.md`).
- Whether to open a pull request for the Linux branch.

## Not started

- Phase 3: M3.1 to M3.4, the `reveal_*` tools and the tokenized CLI; MP5,
  polkit on Linux, with it.
- Phase 4 on macOS (M4.1), and OCR on both systems (M4.2; Tesseract on
  Linux).
- Phases 5 to 7.
- M1.1 host tests, a Cowork cloud task among them from 2026-10-06; M1.2,
  the full off-mode live run over 18 tools.

## Accepted limits

Recorded so they are not mistaken for open work:

- On Linux before 6.12, a reader taken over by a document can signal the
  user's other processes: denial of service only.
- A Drive CLI swapped in between its check and its run is not seen (R9).
- On Linux, pandoc cannot read the old `.doc` format.
- Until OCR, a scanned PDF has no text in aliases mode.
- On Linux, any program of the user reads protonctl's Secret Service items
  once the collection is unlocked.
- The Linux tests need `poppler-utils` and `pandoc`; the full gate also
  needs `dbus` and `gnome-keyring`.
