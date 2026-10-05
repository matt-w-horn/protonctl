[RFC-0001](../rfc-0001.md) › Background

# Background

## Proton's programmatic surfaces (researched 2026-10-02)

| Surface | Status | What matters here |
|---|---|---|
| Drive SDK (`ProtonDriveApps/sdk`, MIT; JS and C#) | Personal non-commercial use allowed; "not yet ready for third-party production use" | No login or session layer. Requires an honest `x-pm-appversion`; masquerading as Proton's clients is forbidden. Clients get rate-limited for "frequent recursive traversals of the file tree". A new crypto model is targeted for "end of 2026/early 2027". |
| Drive CLI 0.8.0 (2026-08-13) | Official | Browser sign-in; session in the Keychain; `--json`; DEBUG logs by default; no search command; no interface-stability promise. Community reports: exit 0 on per-item failures, random list order, `database is locked` under parallel calls. |
| Mail Bridge 3.27 (GPL-3.0) | Official, paid plans | IMAP/SMTP on 127.0.0.1; encrypted local cache; IMAP SEARCH (BODY/TEXT too) runs locally as raw-byte substring scans; labels as `Labels/…`, folders as `Folders/…`; `--noninteractive` runs headless after an interactive first login. No calendar or contacts. |
| Calendar | No API: "Proton Calendar doesn't support CalDAV"; "doesn't support third-party apps" | A Full-view share link is read-only: "you grant Proton Mail temporary access to the calendar. A URL is generated that contains the key". Changes take up to 8 hours to appear. |
| Contacts | No API, no CardDAV | vCard/CSV import and export only. |
| Proton Rust crates | `proton-crypto`, `proton-srp`, `proton-crypto-account`: MIT | Would back a direct-API client if Q1 is revisited. `muon` (Proton's Rust API client) declares no licence. |
| Terms of Service (2026-06-23) §2.10 | | Automation "is permitted provided that the resulting traffic remains indistinguishable from the standard client behavior of human users". |
| Official MCP server | None | Prior art: `fbossiere/proton-safe-mcp` (Bridge, draft-only), `googlarz/proton-drive-mcp` (wraps the Drive CLI), `cheeseandcereal/proton-cal` (internal API). |

## Claude hosts

- Local stdio MCP servers run on the macOS host as Claude Desktop child
  processes, never inside Cowork's VM ("we also moved local MCP servers outside
  the VM"). Cowork's shell is a Linux VM or cloud container that cannot reach
  the host, so a skill driving a CLI through Bash cannot work there.
- New Pro/Max Cowork tasks run in the cloud from 2026-10-06; local MCP servers
  "work through the desktop app only". Whether a cloud session reaches a host
  server is a live test ([section 9](09-rollout.md)).
- Inferred from Claude Desktop's code: tool calls time out at 180 s, tools are
  re-approved when their definitions change, `outputSchema` is not forwarded.
- On Linux, Claude Code runs, and Claude Desktop with Cowork has been in
  beta since the week of 2026-06-29 ([section 11](11-platforms.md#linux-availability-mp0-findings-2026-10-04)).
- Claude Code: permission rules match tool names, not arguments;
  `_meta["anthropic/requiresUserInteraction"]` prompts on every call; it
  keeps every tool result in the session's transcript under
  `~/.claude/projects/`; it speaks
  the pre-2026 handshake to stdio servers (spec 2025-11-25); descriptions are
  capped at 2,048 characters. Elicitation works in Claude Code; Cowork is
  reported to declare it without rendering it, so protonctl does not use it.

## Interface conventions

protonctl follows the conventions of Claude's Gmail (29 tools), Calendar (9)
and Drive (11) connectors, which Google runs: snake_case verb-noun tools
(`search_threads`, `get_thread`, `search_files`, `read_file_content`),
camelCase parameters, `pageSize`/`pageToken`/`nextPageToken` inside tools,
and descriptions that name the sibling tool to call first. It does not copy
`send_message`, `reply`, `forward`, `share_file`, or base64 file content.
In aliases mode (Phase 2) it departs from them in one way: results carry
handles and aliases in place of Proton IDs and names ([section 6](06-privacy.md)), and Drive
tools take a `fileId` handle, as Google's Drive connector does, in place of
a path.

## Design principles

1. Bytes must not transit the model: returning file content as base64
   through the model is slow and expensive.
2. Report per-item outcomes with distinct statuses; the CLI's `skippedItems`
   counts unsupported files and duplicates together.
3. In aliases mode, raw content leaves one item at a time, and only after
   the user approves it on the Mac (R18). This replaced "destructive operations take explicit
   IDs and a dry run first" when protonctl became read-only.
4. One command lists every grant the tool holds, and one removes them,
   naming the steps for any grant that only the service itself can revoke.
5. Official clients beat browser automation and File Provider content reads:
   a browser tab in the background can stall, and a read of a cloud-only
   (dataless) file waits on a download that can stall or time out.
6. The official CLI keeps local state, so calls to it are serialized.
7. Least disclosure when chosen: in aliases mode, results carry aliases
   and hints, not names, and steer Claude toward topics and references
   before raw content. A new install has no mode until the user picks one
   (Q27).
8. Stable without state: aliases, references, handles and keyed digests
   derive from one local key, so they stay valid for months and no map of
   them exists on disk. Stability is also linkability: a name paired with
   its alias once is paired in every transcript under the key (Q21).
9. One choice, not many: a protection that costs the user nothing is not a
   setting, and the one choice that changes what Claude sees is a single
   setting for every service and host ([section 4, Settings](04-design.md#settings)).
10. A portable core: platform-specific code sits behind one boundary, so
    everything else builds and tests on any Unix, a cloud container
    included ([section 11](11-platforms.md)).

---

[Contents](../rfc-0001.md#contents) · [1. Goals and non-goals →](01-goals.md)
