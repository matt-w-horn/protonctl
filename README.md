# protonctl

A local MCP server and CLI that lets Claude (Claude Code, Claude Desktop and
Cowork) read Proton Mail, Calendar and Drive through Proton's own apps. It
holds no Proton password or keys, cannot send mail, share files or delete
anything permanently, and keeps its secrets in the macOS Keychain. Design and
requirements: [docs/rfc-0001.md](docs/rfc-0001.md).

## What works now

| Service | Through | Tools |
|---|---|---|
| Calendar (read-only) | a Full-view share link, fetched by `/usr/bin/curl` | `list_calendars`, `list_events`, `search_events`, `get_event` |
| Drive (read-only) | the Proton Drive app's local folder, and the official Proton Drive CLI for content; without the app, listing goes through the CLI and search is off | `search_files`, `list_folder`, `get_file_metadata` (with `digests: true`, Proton's node IDs, the SHA-1 Proton stored at upload, and local digests), `read_file_content` (the text of text files, PDFs and Word, RTF and OpenDocument documents, a page at a time, and images), `download_file` (any file, with its SHA-256 and SHA-1, saved or returned inline), `list_drive_tree` (everything under a folder, a page at a time), `export_drive_manifest` (the same inventory written to the export folder) |
| Mail (read-only) | Proton Mail Bridge, over local IMAP with its certificate pinned | `search_threads` (Gmail-style queries, one row per thread, optional snippets), `count_messages` (totals, or groups by sender, domain or recipient, in one call), `get_thread` (quoted replies removed), `get_message`, `list_labels`, `get_attachment` (text, PDF and document text, or images, with its SHA-256 and SHA-1) |

`get_status` lists what protonctl can reach and every secret it holds.

Content comes back in the tool result wherever it can, so no folder is
needed to read it. Text pages by `offset` and `maxChars` (20,000 characters
unless asked otherwise), and each result says where the next page starts.
PDF text comes from macOS's PDFKit and Word, RTF and OpenDocument text from
`textutil`, both part of macOS, so there is nothing more to install. Images
come back as image content. A file saved by `download_file`, or an
attachment that is neither text nor an image, goes to a private folder and
is removed after an hour, or when the server stops. With `export: true` it
goes to the export folder instead (see Configuration). With `inline: true`
its bytes come back in the result, base64, which not every host accepts yet.

## Install

```sh
cargo install --path . --locked --root ~/.cargo
```

The commands below use the full path, in case `~/.cargo/bin` is not on your
PATH. Each rebuild changes the binary's ad-hoc signature, so macOS asks once
more for each Keychain item (the calendar link, the Bridge password) the
next time protonctl reads it. Enter the login keychain password and choose
Always Allow; that lasts until the next rebuild.

## Add a calendar

In the Proton web app, open the calendar's sharing settings and create a
Full-view link used only by protonctl. Copy it, then:

```sh
pbpaste | ~/.cargo/bin/protonctl setup calendar --id personal
```

The link goes to the Keychain (service `protonctl`, account
`calendar/personal`) and the calendar's id and name go to
`~/.config/protonctl/config.toml`. Proton's server decrypts the calendar
whenever the link is fetched, and changes can take up to 8 hours to appear.

## Add mail

Proton Mail Bridge (a separate app from the Proton Mail desktop app) must be
installed and signed in, with **Show All Mail** on and **Connection mode** set
to **SSL**, both under Advanced settings. It need not stay open: when it isn't
running, protonctl opens it in the background and waits about 10 seconds. Select your account in Bridge and copy
its password: the Bridge password, not your Proton password. Then run this
with the username Bridge shows:

```sh
pbpaste | ~/.cargo/bin/protonctl setup mail --address you@proton.me
```

The first connection pins Bridge's TLS certificate, and later connections
refuse any other. If you reinstall Bridge, its certificate changes: delete
`[mail]` from the config and run setup again.

## Connect Claude

Claude Code:

```sh
claude mcp add --scope user proton -- "$HOME/.cargo/bin/protonctl" serve
```

Claude Desktop and Cowork: quit the app, then add a `proton` entry under
`mcpServers` in `~/Library/Application Support/Claude/claude_desktop_config.json`.
That file already holds the app's own settings, so merge into it rather than
replace it. This does the merge and leaves the old file beside it as `.bak`:

```sh
python3 -c 'import json, os, shutil; p = os.path.expanduser("~/Library/Application Support/Claude/claude_desktop_config.json"); c = json.load(open(p)); shutil.copy(p, p + ".bak"); c.setdefault("mcpServers", {})["proton"] = {"command": os.path.expanduser("~/.cargo/bin/protonctl"), "args": ["serve"]}; json.dump(c, open(p, "w"), indent=2, ensure_ascii=False)'
```

Then open the app again.

In Claude Code, these rules allow the read tools without a prompt, and leave
`export_drive_manifest`, which writes a file, on ask: `mcp__proton__get_*`,
`mcp__proton__list_*`, `mcp__proton__search_*`, `mcp__proton__count_*`,
`mcp__proton__read_*`, `mcp__proton__download_*`.

## Check, and revoke

```sh
~/.cargo/bin/protonctl doctor
```

```sh
~/.cargo/bin/protonctl logout
```

`logout` deletes protonctl's Keychain items. A calendar link keeps working for
anyone who has it until you delete it in the calendar's sharing settings in
the Proton web app. The Proton Drive CLI has its own session, which this ends:

```sh
proton-drive auth logout
```

## Configuration

`~/.config/protonctl/config.toml` holds no secrets. The format is documented
at the top of `src/config.rs`; for example, `[drive] exclude = ["/Private"]`
hides a folder from every result.

### Export folder

An agent whose shell runs elsewhere (Cowork's VM) cannot read protonctl's
private download folder. Text, documents and images reach it in the tool
result; for whole files, name an export folder, and `download_file` and
`get_attachment` with `export: true`, and `export_drive_manifest`, write
there instead:

```toml
[export]
folder = "/Users/you/protonctl-export"
```

Files land at `drive/<Drive path>`, `mail/<messageId>/<index>-<name>` and
`manifests/drive-<time>.jsonl`, and stay until you delete them. The folder
must be an absolute path outside the Proton Drive app's folder (writing there
would upload to Proton) and outside `~/Library/Caches/protonctl`. Pick one no
sync service mirrors, unless you want that, and connect it to Cowork to give
agents there the files.

## Development

```sh
scripts/check.sh
```

That runs every gate the pre-commit hook runs: fmt, clippy with `-D warnings`,
the tests, `cargo deny check`, and a line-coverage floor through
`cargo-llvm-cov` and Homebrew's `llvm` (`brew install cargo-llvm-cov llvm`),
which prints coverage per file. Unused dependencies are checked by hand, now
and then, with `cargo machete` (`cargo install cargo-machete --locked`).
Inside Claude Code's Bash sandbox `~/.cargo` is not writable, so point Cargo
elsewhere first:

```sh
export CARGO_HOME="$TMPDIR/cargo-home"
```

After an install, this runs every tool once against your real calendar,
mail and Drive, as the Claude app does, and prints only outcomes and counts.
Run it from a normal terminal; it uses the installed binary, so a Keychain
approval given to the Claude app covers it too:

```sh
python3 scripts/live-check.py
```

Also expect `codesign` to report a valid Proton binary as "modified" there;
judge the Drive CLI's signature with the `doctor` command above, run from a
normal terminal. The mail tests start a fake Bridge on a local port, which the
sandbox refuses unless `sandbox.network.allowLocalBinding` is true.
