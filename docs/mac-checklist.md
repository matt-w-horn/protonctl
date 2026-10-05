# Mac checklist for Phase 2

What the Linux build could not check. Work through it on the Mac after
pulling, in order. When an item passes, delete it; when the list is empty,
delete this file. Milestone states live in [RFC section 9](rfc-0001/09-rollout.md).

## 1. Build and gates

1. Run `scripts/check.sh`. It adds what Linux skips: the PDFKit and
   `textutil` tests, and coverage through Homebrew's `llvm`.
2. Run `cargo install --path . --locked --root ~/.cargo`.

## 2. Before a mode is chosen (Q27)

1. Run `~/.cargo/bin/protonctl status`. Expect `privacy.mode` `"unset"`.
2. Run `~/.cargo/bin/protonctl doctor`. Expect it to fail on the privacy
   line.
3. In Claude Code, call `get_status`. Expect the error
   `privacy_mode_unset`.

## 3. The Keychain code (new, linted on Linux, never run)

The functions are `secret_set_with_comment` and `secret_comment` in
`src/platform/macos.rs`.

1. Run `protonctl setup privacy`. Expect `"key": "created"` and
   `"mode": "aliases"`.
2. Run `security find-generic-password -s protonctl -a privacy-mode`.
   Expect the `icmt` attribute (the comment) to be `aliases`.
3. Run `security find-generic-password -s protonctl -a privacy-key`.
   Expect `icmt` to be a 32-character hex key ID.
4. Restart Claude Code, then call `get_status` twice. Note whether macOS
   asks for Keychain access: reading the mode reads only the comment and
   should not ask; loading the key asks once per rebuild (Always Allow).
5. With that server still running, run `protonctl setup privacy --off`.
   Expect it to ask on the terminal. Then call any tool: expect
   `privacy_mode_changed`. Restart Claude Code: expect off mode.
6. Run `protonctl setup privacy` again. Expect `"key": "kept"`.
7. With a server running in aliases mode, note one alias, run
   `protonctl rotate-key` (it asks), and repeat the call. Expect a
   different alias, with no restart.
8. Run `protonctl logout`. Expect it to ask, then to list `privacy-key`
   and `privacy-mode` among the deleted items. The next call gives
   `privacy_mode_unset`.

## 4. Drive setup (M1.4)

1. Run `~/bin/proton-drive auth login`, then `protonctl setup drive`.
   Expect `[drive]` in the config, with the app's folder.
2. Run `protonctl setup drive` again. Expect a refusal that names the
   config file.
3. In off mode, call `download_file` twice in Claude Code; it always
   runs the CLI. The CLI's signature is now checked before every run
   (Q24), one `codesign` call each. Expect both calls to pass, and note
   how much time the check adds per call.

## 5. Live checks (M1.1, M1.2, M2.10)

`scripts/live-check.py` now runs in either mode. Its aliases-mode
branches have never run.

1. In off mode, run `python3 scripts/live-check.py` from a normal
   terminal. Expect "all calls passed".
2. In aliases mode, run it again. Expect "all calls passed", with no raw
   addresses, links, local paths or raw digests counted.
3. In Claude Code and in Cowork (M1.1, M1.2), ask one question per
   service in each mode.
4. Repeat the role-play of
   [Appendix C](rfc-0001/appendix-c-roleplay.md) on real results in
   aliases mode. Record only its findings.

## 6. Still to build on the Mac

1. M2.8: cloud-only Drive reads in aliases mode through a per-process RAM
   disk (Q14). Until then they give `invalid_argument`, and without the
   Proton Drive app no Drive file can be read in aliases mode. Build it as
   `memory_dir` in `src/platform/macos.rs`, which now returns that error;
   `content::memory_folder` and the Drive read already use it, as the
   Linux version does.
2. Q12: a self-signed code-signing certificate, so Keychain approvals
   survive rebuilds.

## 7. Checks the container could not reach

1. The EFF large word list's licence. `src/privacy/words.rs` says
   Creative Commons Attribution 3.0 US; eff.org was blocked here. Confirm
   it on EFF's "new wordlists" page, and add the licence link to the
   header of `words.rs` if attribution needs it.

## 8. Open questions that need a Mac

- Q13: the sandbox profile for PDFKit and `textutil` (Phase 4).
- Q15: whether a LocalAuthentication prompt appears under each host and
  inside Claude Code's sandbox (Phase 3).
- Q17: the Claude Code sandbox settings (Phase 3).
