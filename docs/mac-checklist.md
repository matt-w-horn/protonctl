# Mac checklist for Phase 2

What the Linux build could not check. Work through it on the Mac after
pulling, in order. When an item passes, delete it; when the list is empty,
delete this file. Milestone states live in [RFC section 9](rfc-0001/09-rollout.md).

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
host restart, except section 1, which needs only a Mac.

## 1. After pulling the Linux work

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

## 2. The Keychain code, through the hosts

1. With a server still running, run `protonctl setup privacy --off` in
   a terminal and confirm. Then call any tool: expect
   `privacy_mode_changed`. Restart Claude Code: expect off mode.
2. Run `protonctl setup privacy`, restart, and note one alias. Run
   `protonctl rotate-key` (it asks), and repeat the call. Expect a
   different alias, with no restart.

## 3. Live checks (M1.1, M1.2, M2.10)

`scripts/live-check.py` runs in either mode; it has passed in aliases mode.

1. In off mode, run `python3 scripts/live-check.py` from a normal
   terminal. Expect "all calls passed".
2. In Claude Code and in Cowork (M1.1, M1.2), ask one question per
   service in each mode.
3. Repeat the role-play of
   [Appendix C](rfc-0001/appendix-c-roleplay.md) on real results in
   aliases mode. Record only its findings.

## 4. Last: logout

`protonctl logout` deletes every protonctl Keychain item, the calendar
links and the Bridge password included, so `setup calendar` and
`setup mail` must run again afterwards. Run it only when ready to redo
them. Expect it to ask, then to list `privacy-key` and `privacy-mode`
among the deleted items. The next call gives `privacy_mode_unset`.

## 5. Open questions that need a Mac

- Q13: the sandbox profile for PDFKit and `textutil` (Phase 4).
- Q15: whether a LocalAuthentication prompt appears under each host and
  inside Claude Code's sandbox (Phase 3).
- Q17: the Claude Code sandbox settings (Phase 3).
