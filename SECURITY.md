# Security policy

## Report a vulnerability

Report a vulnerability privately, through GitHub's private vulnerability
reporting on this repository. On the repository's **Security** tab, choose
**Report a vulnerability**, or open
[the report form](https://github.com/matt-w-horn/protonctl/security/advisories/new)
directly. Do not report a vulnerability in a public issue.

In the report, include:

- what an attacker can do, and what the attacker needs first;
- the steps that show the problem;
- the protonctl version (`protonctl --version`), the macOS version, and
  the privacy setting (off or aliases).

Use synthetic data in the report. Never include real mail, files, calendar
links, passwords or Keychain contents.

## Get help, or report a bug

Use [GitHub issues](https://github.com/matt-w-horn/protonctl/issues). Before
you paste output from protonctl or a Claude app, remove names, addresses,
file names and links from it.

## Scope

In scope:

- protonctl: the program in `src/`, its command line and MCP server, and
  `scripts/install.sh`.
- The plugin files: `.claude-plugin/plugin.json` (the plugin's manifest,
  which declares the `proton` server), `.claude-plugin/marketplace.json`
  and `scripts/serve`.
- A security claim in the [README](README.md) or in
  [RFC-0001](docs/rfc-0001.md) that the code does not meet.

Out of scope:

- Proton's apps and services: Proton Mail Bridge, the Proton Drive app,
  the Proton Drive CLI, the Proton web app, and Proton's servers. Report
  these to Proton.
- Claude hosts and Anthropic's services: Claude Code, Claude Desktop,
  Cowork and claude.ai. Report these to Anthropic, as its
  [Responsible Disclosure Policy](https://www.anthropic.com/responsible-disclosure-policy)
  describes.
- Attacks that need root on the Mac, or access to the Proton account
  itself.

[RFC-0001, section 5](docs/rfc-0001/05-security.md) and the
[security and privacy review](docs/rfc-0001/security-privacy-review.md)
list the threats that protonctl controls and the risks that it accepts.
