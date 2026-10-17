[RFC-0001](../rfc-0001.md) › 1. Goals and non-goals

# 1. Goals and non-goals

Goals:

1. Search and read Proton Mail and Drive, and read Proton Calendar, from Claude
   Code, Claude Desktop and Cowork on macOS, and on Linux from Claude Code
   and, where its beta loads local servers, Claude Desktop
   ([section 11](11-platforms.md)), with tools shaped like Claude's Gmail,
   Calendar and Drive connectors.
2. Serve results as Claude's Google connectors do, read-only, with every
   protection that costs the user nothing ([section 6](06-privacy.md)). This goal
   replaced "safe writes". It then became a choice between aliases mode and
   off mode (Q26). Aliases mode was later withdrawn
   ([Q43](10-open-questions.md)), and off mode is the only behavior.
3. Keep Proton's model intact: keys and decryption stay inside Proton's clients.
4. Small enough to review in one sitting: after the privacy layer was
   removed, `src/` held about 16,000 lines, unit tests included.
5. No daemon of ours: the server lives as long as the Claude Code session or
   the Desktop app.

Design target: the maintainer is the only user. protonctl suits people who
want a read-only stand-in for Claude's Google connectors. Every protection
that costs the user nothing applies, and there is no mode to choose. Aliases
mode, which once served people at risk of targeted attacks, was withdrawn by
[Q43](10-open-questions.md).

Non-goals: sending mail, sharing, public links and invitations (never); any
write to the account, drafts, labels, moves, flags, trash and uploads
included (since Q5; [RFC-0002](../rfc-0002.md) proposes writes
under a policy the user sets); filters and account settings; Proton Pass;
Contacts; the content of Docs and Sheets, which no official client reads
(the Drive CLI skips them); Photos (RFC-0002, Q51); VPN, Wallet; other
users; Windows.
Hiding names or content from the host's model provider: what a result
holds reaches the provider (Q43). Protection against code running as root,
or against anyone with access to the Proton account.

---

[← Background](background.md) · [Contents](../rfc-0001.md#contents) · [2. Requirements →](02-requirements.md)
