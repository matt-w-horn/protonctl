# To do

Every open item in one place: bugs, missing tests, reviews, the checks
that need a Mac or a Linux desktop, decisions, and work not started.
Documentation that states something wrong is a bug. Milestone states live in
[RFC section 9](rfc-0001/09-rollout.md); this file lists what is left.
When an item is done, delete it; the commit that does it says so.

Where: **here** can be done in a Linux container; **Mac** and **Linux
desktop** need that machine and, mostly, a person; **decision** needs the
maintainer.

## Bugs

- B15 (here) In aliases mode, short forms of a name pass raw
  ([section 9](rfc-0001/09-rollout.md#phase-2-defects-found-on-real-results), defect D2).
- B16 (here) In aliases mode, initials pass raw
  ([section 9](rfc-0001/09-rollout.md#phase-2-defects-found-on-real-results), defect D3).
- B17 (here) In aliases mode, misspelled names pass raw, from typing and
  from OCR
  ([section 9](rfc-0001/09-rollout.md#phase-2-defects-found-on-real-results), defect D4).
- B18 (here) In aliases mode, project names pass raw, in text and in Drive
  paths and names
  ([section 9](rfc-0001/09-rollout.md#phase-2-defects-found-on-real-results), defect D5).
  Projects need an entity type and a source of names.
- B19 (here) In aliases mode, one entity gets several aliases
  ([section 9](rfc-0001/09-rollout.md#phase-2-defects-found-on-real-results), defect D6).
  Forms of one name, one organization or one project should share an
  alias, or be linked (`maybeSameAs`, M5.2).
- B20 (here) In aliases mode, organizations and products are typed
  `person`, or pass raw
  ([section 9](rfc-0001/09-rollout.md#phase-2-defects-found-on-real-results), defect D7).
- B21 (Mac) A folder made online-only in the Drive app lists as empty, with
  no note, in both modes
  ([section 9](rfc-0001/09-rollout.md#phase-2-defects-found-on-real-results), defect D8).
  It should be listed through the CLI, or say that its contents are not
  on this computer.
- B23 (here) In aliases mode, a phone or card number written in another
  script's digits passes raw: "+١ ٤١٥ ٥٥٥ ٠١٢٣" and "٤١١١ ١١١١ ١١١١ ١١١١"
  (Arabic-Indic) are not found, while their ASCII forms are. `phone` in
  `src/privacy/detect/pattern.rs` counts ASCII digits only, and the card
  check reads digits with `char::to_digit`, which is ASCII only. An SSN in
  such digits is found. A fix needs each script's digit values.
- B24 (here) In aliases mode, a 40- or 64-hex digest or a Proton message
  ID written in free text passes raw: no detector in
  `src/privacy/detect/pattern.rs` covers them. R16 and R17 cover them in
  fields, which the leak test checks. `scripts/live-check.py` flags any
  such run in any text, so a live run and the pipeline disagree.
- B25 (here) In aliases mode, a Chinese or Japanese name inside running
  text in that script is not found: a dictionary match must have a
  character that is not a letter or digit on each side (`bounded` in
  `src/privacy/detect/mod.rs`), and such text has no spaces.

## Tests the RFC plans that do not exist

- T11 (here, larger) A hermetic IMAP test against Dovecot in podman.
- T12 (later) A prompt-injection drill, about 5 Claude runs.
- T14 (not testable here) The x32 seccomp rule, which needs unsafe code or
  a kernel built with x32; and that secrets are wiped on drop (R2), which
  needs a look at freed memory.
- T15 (here) An evaluation of the privacy layer
  ([section 9](rfc-0001/09-rollout.md#phase-2-defects-found-on-real-results), defect D1):
  run aliases mode over a labeled synthetic corpus that holds each name,
  organization and project in every form of B15 to B20, and report, per
  form and per entity type, what came back raw, in part or whole, and
  what was given two aliases. Each fix of B15 to B20 must move its
  number.
- T18 (here) `scripts/live-check.py` in aliases mode does not check phone
  numbers, that every `entities` entry has a `ref`, or that a second
  server run gives the same aliases for the same search, as section 7
  plans.

## Reviews

- R1 (decision) Q25: `/security-review` on the Phase 2 changes has no
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

### M6. Before M4: the privacy key does not synchronize (R20)

R20 expects the `privacy-key` item not to synchronize, because protonctl
writes it to the file-based login keychain, which iCloud does not sync.
M2.1 recorded no check of this. Confirm it before M4 deletes the item.

## Needs a Linux desktop

- L1 MP2's live check: mail and calendar, with Bridge as a systemd user
  unit.
- L2 MP3's live check: Drive through the real CLI, signed in.
- L3 Whether Claude Desktop's Linux beta loads local MCP servers, and how
  Cowork's VM reaches them (section 11).
- L4 A locked keyring with the unlock prompt on screen. Without a display,
  every read fails at once (`a_locked_keyring_fails_at_once_without_a_prompt`
  in `src/platform/linux.rs`). With one, a read waits until the prompt is
  answered: under a virtual display, one was still waiting at 60 s. A
  call answers `timeout` at its 150 s, though its read waits on; the
  server's read of the privacy setting at start (`Privacy::stored`, from `App::load` in
  `src/main.rs`) has no limit, so the server answers nothing until then.
  Check what Claude Code and Claude Desktop do with a server that waits
  so at start, and whether a first `setup` shows the prompt; then decide
  whether the start-up read needs a limit.
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
- Aliases mode's first Drive CLI run creates an empty
  `~/.cache/protonctl/cli.lock` (`lock_path` in `src/drive/cli.rs`): no
  content, so R10 holds, but a new file in the home folder. Keep it, or
  move the lock into the memory folder. The no-disk test allows it while
  it is empty.

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
