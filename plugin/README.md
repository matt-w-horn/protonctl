# protonctl plugin for Claude Code and Cowork

This plugin registers `protonctl` as the `proton` local, read-only MCP server in Claude Code and Cowork (in sessions that run on your Mac), letting Claude answer questions about your Proton Mail, Proton Calendar and Proton Drive through Proton's own apps on your computer. It cannot send, reply, forward, draft, share, label, move, flag or delete.

**protonctl is unofficial. It is not affiliated with, endorsed or sponsored by Proton AG.**

## Install

1. Install and sign `protonctl` itself first (the plugin bundle holds only the launcher):

   ```sh
   brew install matt-w-horn/tap/protonctl
   protonctl-sign
   ```

   For source builds on macOS or Linux, and to connect Proton Mail Bridge, a calendar share link and Proton Drive, follow the [full setup guide](../README.md).

2. Install the plugin in Claude Code:

   ```text
   /plugin install protonctl --marketplace matt-w-horn/protonctl
   ```

   For Cowork, add the marketplace `matt-w-horn/protonctl` in Claude Desktop under **Customize > Plugins**, then restart Claude Code and Claude Desktop.

## Example prompts

- "Which threads this week mention the invoice, and who sent them?" (`search_threads`, `get_thread`)
- "What is on my calendar next Tuesday, and does anything overlap?" (`list_events`)
- "Find the lease PDF in my Drive and summarize its renewal terms." (`search_files`, `read_file_content`)

## Links

- Documentation: [README.md](https://github.com/matt-w-horn/protonctl#readme)
- Privacy policy: [PRIVACY.md](https://github.com/matt-w-horn/protonctl/blob/main/PRIVACY.md)
- Security and support: [SECURITY.md](https://github.com/matt-w-horn/protonctl/blob/main/SECURITY.md) and [GitHub issues](https://github.com/matt-w-horn/protonctl/issues)
- License: Apache-2.0 ([LICENSE](LICENSE))
