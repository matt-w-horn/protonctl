# Privacy policy

Last changed: 2026-10-05.

This policy covers protonctl: the program, its command line, and its
Claude Code plugin. protonctl runs on your Mac and lets Claude read your
Proton Mail, Calendar and Drive. It is unofficial and not affiliated with
Proton AG. Proton's apps and services, and Anthropic's Claude apps and
services, have their own privacy policies. This policy does not cover them.

## Summary

- protonctl runs on your Mac. It sends nothing to its developer, and the
  developer runs no server for it.
- It reads your mail, events and files when a Claude app calls one of its
  tools, or when you run one of its commands.
- It gives each result to the Claude app that asked for it. That app sends
  the result to Anthropic, under the terms of your Claude account.
- It is read-only. It cannot send, share, draft, label, move or delete.
- It keeps its secrets in your Mac's Keychain and its settings in one file.
  The table under [What protonctl keeps on your Mac](#what-protonctl-keeps-on-your-mac)
  lists everything it keeps.

## What protonctl reads

protonctl reads your Proton data only when a Claude app calls one of its
tools, or when you run one of its commands. It reads through Proton's own
apps and a calendar link:

- **Mail**, through Proton Mail Bridge on your Mac. protonctl opens
  mailboxes read-only.
- **Calendar**, through a Full-view share link that you make for
  protonctl.
- **Drive**, from the Proton Drive app's folder on your Mac, and through
  the official Proton Drive CLI.

What it reads depends on the call. A search returns details such as
senders, subjects, dates and file names, and short text snippets only when
the call asks for them. A read returns the content of one message, event or
file. protonctl never shows or reads a Drive folder that you exclude in the
settings.

In aliases mode, protonctl also reads the display names in your mail's
headers and the names of your calendar's attendees, once for each server
process. It holds them in memory only, and uses them to find those names in
other results and replace them with aliases.

The [README](README.md#what-works-now) lists every tool.

## Where results go

1. protonctl gives each result to the Claude app on your Mac that called
   the tool: Claude Code, Claude Desktop or Cowork. protonctl sends it to
   no other program or service.
2. The Claude app sends the result to Anthropic as part of your
   conversation. Anthropic processes it under your Claude account's terms
   and settings. Before you connect mail, check your account's setting on
   the use of your chats to train models.
3. The Claude apps also keep results on your Mac. Claude Code keeps each
   session's transcript, tool results included, under
   `~/.claude/projects/`, for 30 days by default. Claude Desktop keeps each
   server's log under `~/Library/Logs/Claude/`, and can keep whole messages
   there. protonctl cannot delete these files.
4. If a session has other tools, such as web fetch, a shell or another
   connector, Claude can pass a result on through them. protonctl cannot
   control this.
   [RFC-0001, section 5](docs/rfc-0001/05-security.md#threats-and-controls)
   describes this risk.

protonctl cannot recall a result once it has left. A change to the privacy
setting does not change results that Claude already has.

## What protonctl keeps on your Mac

| What | Where | How long |
|---|---|---|
| Secrets: the Bridge password, each calendar's share link, the privacy setting, and the privacy key (made when you choose aliases mode) | the login Keychain, service `protonctl` | until `protonctl logout` deletes them |
| Settings, with no secrets: calendar ids and names, the Bridge address and port, the pin of Bridge's certificate, Drive paths and exclusions, the export folder, the time zone | `~/.config/protonctl/config.toml` | until you delete it |
| In off mode: files that `download_file` saves, and mail attachments that are neither text nor an image | a private folder for each server process, under `~/Library/Caches/protonctl/downloads/` | each is removed after an hour; the folder is removed when the server stops |
| In off mode: files that you export (`export: true`, and `export_drive_manifest`) | the export folder, if you set one ([README](README.md#export-folder)) | until you delete them |
| In aliases mode: Drive files that are only in the cloud, while protonctl reads them | a RAM disk that protonctl makes for itself, under `~/Library/Caches/protonctl/memory/` | each file is deleted once read; the RAM disk is removed when the process exits |
| An empty lock file, which stops two protonctl processes from running Proton's Drive command-line tool at once; it holds nothing | `~/Library/Caches/protonctl/cli.lock` | until you delete it |
| Files that you save with the commands `protonctl drive get` and `protonctl mail attachment` | the folder that `--out` names, or else the current folder | until you delete them |
| The signing identity that `scripts/install.sh` makes: a certificate named `protonctl`, and its key | the login keychain | until you delete it |

If a server is killed or crashes, its download folder stays until you
delete it. A RAM disk that is left behind is gone after the Mac restarts.

protonctl keeps no record of the calls it answers. It writes warnings and
errors to its log (stderr), which the Claude apps keep. That log carries no
content, names, queries or IDs (requirement R25 in
[RFC-0001, section 2](docs/rfc-0001/02-requirements.md)).

Proton's apps keep their own data: Bridge's encrypted store, the Proton
Drive app's folder, and the Drive CLI's cache. protonctl runs the Drive CLI
so that it logs errors only. The CLI still writes metric events (counts,
sizes and timings, with no names or IDs) to
`~/Library/Logs/proton-drive-cli`.

## How long data stays

- Secrets and settings: until you remove them (see
  [How to remove everything](#how-to-remove-everything)).
- Downloads: an hour, or until the server stops.
- Exports, and files that you save with protonctl's commands: until you
  delete them.
- The RAM disk in aliases mode: until the process exits.
- In memory, at most until the server stops: each calendar, for
  15 minutes after it is fetched; the list of Drive names that search
  uses; and, in aliases mode, the names from your mail and calendar.

The Claude apps and Anthropic keep results for as long as their own
settings and terms say.

## What goes to Proton

- protonctl's own network connections go only to your Mac (127.0.0.1),
  where Bridge listens.
- To read a calendar, protonctl has `/usr/bin/curl` fetch its share link
  from Proton's server over HTTPS. The link goes to curl on its input,
  never on its command line. Proton's server decrypts the calendar each
  time someone fetches the link. A server keeps a fetched calendar for
  15 minutes, and fetches it again only when a call needs it after that.
- Bridge and the Drive CLI talk to Proton's servers themselves, with
  Proton's end-to-end encryption and under Proton's terms. protonctl asks
  them only for what a call needs.

## What the developer receives

Nothing from protonctl. protonctl has no telemetry, sends no crash
reports, and does not check for updates. The developer runs no server for
it. What the plugin runs is a launcher that starts protonctl on your Mac.

For the plugin in Anthropic's plugin directory, Anthropic shows the
developer installs, versions, how often the plugin's server runs, and error
rates. These figures come from Anthropic, under Anthropic's terms, not from
protonctl.

If you write to the developer through GitHub, in an issue or a
vulnerability report, the developer receives what you write. Do not
include real mail, files, calendar links or passwords.

## The privacy setting

protonctl answers no call until you choose one of two settings
([README](README.md#choose-the-privacy-setting)):

- **Off**: results as Proton's apps show them, names included.
- **Aliases**: names, addresses, phone and account numbers, codes and
  passwords appear as aliases, such as `amber-falcon-river`. IDs and Drive
  paths become opaque handles. The server writes no content to disk, and
  returns images as a type and a reason.

Aliases mode does not find every name yet. A name that appears only in
free text, such as a message body, a subject or a file name, can reach
Claude as written.
[RFC-0001, section 5](docs/rfc-0001/05-security.md#residual-risks) lists
what aliases mode does not hide.

The setting is one Keychain item for every Claude app on the Mac. A change
applies only to results after it.

## How to remove everything

1. Quit Claude Code and Claude Desktop, so that no protonctl server runs.
2. Delete protonctl's Keychain items. The command asks first:

   ```sh
   ~/.cargo/bin/protonctl logout
   ```

3. In the Proton web app, delete each calendar's share link in that
   calendar's sharing settings. Until you do, the link works for anyone
   who has it.
4. End the Drive CLI's session with `proton-drive auth logout`. To end
   Bridge's local access to your mail, sign out in Bridge.
5. Delete `~/.config/protonctl/` and `~/Library/Caches/protonctl/`. Also
   delete the export folder, if you set one, and any files that you saved
   with `protonctl drive get` or `protonctl mail attachment`.
6. Remove the plugin: in Claude Code, through `/plugin`; for Cowork, under
   **Customize > Plugins** in the Claude desktop app. If you added the
   server by hand, run `claude mcp remove proton`, and remove the `proton`
   entry from Claude Desktop's configuration.
7. Delete `~/.cargo/bin/protonctl`. In Keychain Access, delete the
   `protonctl` certificate and its key from the login keychain, under
   **My Certificates**.

protonctl cannot delete what the Claude apps and Anthropic keep. Your
Claude account's controls and the apps' settings cover that.

## Changes to this policy

The date at the top changes with each change to this policy. The
repository's Git history records every change.

## Contact

Ask about this policy, or about protonctl, in
[GitHub issues](https://github.com/matt-w-horn/protonctl/issues). Report a
security problem privately, as [SECURITY.md](SECURITY.md) describes.
