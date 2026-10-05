[RFC-0001](../rfc-0001.md) › 1. Goals and non-goals

# 1. Goals and non-goals

Goals:

1. Search and read Proton Mail and Drive, and read Proton Calendar, from Claude
   Code, Claude Desktop and Cowork on macOS, and on Linux from Claude Code
   and, where its beta loads local servers, Claude Desktop
   ([section 11](11-platforms.md)), with tools shaped like Claude's Gmail,
   Calendar and Drive connectors.
2. Let the user choose what reaches the model provider. In aliases mode,
   disclose as little as possible: results carry stable aliases, and raw
   content leaves one approved item at a time ([section 6](06-privacy.md)). In off mode,
   serve results as Claude's Google connectors do. This goal replaced "safe
   writes" on 2026-10-04 and became a choice the same day (Q26).
3. Keep Proton's model intact: keys and decryption stay inside Proton's clients.
4. Small enough to review in one sitting: on 2026-10-04, about 7,300 lines
   of code and 4,400 of tests, before the privacy layer.
5. No daemon of ours: the server lives as long as the Claude Code session or
   the Desktop app.

Design target: the maintainer is the only user. Aliases mode suits people
who turn on Lockdown Mode or Advanced Data Protection: people at risk of
targeted attacks, malicious files and legal demands for their transcripts.
Off mode suits people who want a read-only stand-in for Claude's Google
connectors. Every protection that costs the user nothing applies in both
modes. A new install has no mode until the user picks one, since a
wrong `off` cannot be recalled (Q27).

Non-goals: sending mail, sharing, public links and invitations (never); any
write to the account, drafts, labels, moves, flags, trash and uploads
included (since 2026-10-04); filters and account settings; Proton Pass;
Contacts; Docs/Sheets content, Photos, VPN, Wallet; other users; Windows.
Anonymization in the legal sense: the privacy layer is pseudonymization plus
data minimization, and it reduces, but does not prevent, inference of
identity from context. Protection against code running as root, or against
anyone with access to the Proton account.

---

[← Background](background.md) · [Contents](../rfc-0001.md#contents) · [2. Requirements →](02-requirements.md)
