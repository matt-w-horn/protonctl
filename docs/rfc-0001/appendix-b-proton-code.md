[RFC-0001](../rfc-0001.md) › Appendix B: Proton's open-source code

# Appendix B: Proton's open-source code (surveyed 2026-10-03)

The question: could protonctl work without Bridge, the Drive app and the
calendar link by using Proton's own libraries? Every repository in
`github.com/ProtonMail` (186) and `github.com/ProtonDriveApps` (11) was listed
through the GitHub API. The candidates' READMEs, manifests, licenses and
source layout were read, and so were the crates they pull from Proton's Rust
registry (`rust-registry.proton.me`, which answers the public).

**Building blocks**

| Repository | Language, license | What it offers | Catch |
|---|---|---|---|
| `clients` (Proton's monorepo; `rust-mail` moved here in June 2026) | Rust; GPL-3.0 at the root, AGPL-3.0 for `project/mail/rust` | The live Rust core of Mail, Calendar, Contacts and account: API types, crypto, iCal, mail search, an event loop, human verification, and `mail-tui`, a terminal mail client built on these crates | `mail-tui` is "an internal testing tool" with no support; the build is Bazel-first and pulls a large graph (Crux, diesel, uniffi) from Proton's registry; AGPL would bind protonctl if it were ever distributed |
| `muon` (Proton's public registry; a fork, `mail-muon` 1.7.1, sits in `clients`' AGPL mail tree) | Rust; no license file or field in 4.1.0 | The Proton API client: SRP sign-in, 2FA, human verification, session refresh, DNS-over-HTTPS fallback, and session forks, which give a browser login (a URL to open) and a code login | The missing license is a gap in the files, not a technical block: Proton's own GPL-3.0 `pass-cli` (the Proton Pass CLI) and `protun` (ProtonVPN) depend on it from the same registry, and `pass-cli` tells the public to build it with `cargo build`. Its README still calls the registry internal, and app identities are Proton products unless built with `other-product` |
| `proton-crypto-rs` | Rust; MIT | `proton-srp`, `proton-crypto` (OpenPGP over rPGP in pure Rust, or GopenPGP through Go) and `proton-crypto-account` (user and address keys) | "Not intended or vetted for general usage outside Proton"; no outside contributions |
| `go-proton-api` | Go; MIT | Bridge's own API library: SRP sign-in with human verification, key unlock, messages, drafts, send, labels, events, calendar events, contact cards, Drive shares, links and blocks | Go, not Rust; signs in with username and password only; not seeking contributors |
| `go-srp`, `gopenpgp`, `go-crypto`, `gosop` | Go; MIT or BSD-3-Clause | The crypto under `go-proton-api` | as above |
| `gluon`, `proton-bridge`, `proton-mail-export` | Go; MIT, GPL-3.0, GPL-3.0 | Bridge's IMAP server, Bridge itself, and the mail export tool | the Bridge route itself, or export only |
| `sdk` (ProtonDriveApps) | TypeScript; MIT | The official Drive SDK, which the `proton-drive` CLI wraps, and the only Proton component with a stated third-party policy: personal, non-commercial use, an honest `external-drive-{name}@{semver}` app version, event-based sync and no frequent tree walks | A new cryptographic model around end-2026 or early 2027 breaks older releases; no Rust port; `sdk-swift` is an empty repository so far, and `dotnet-crypto` (C#, MIT) is the .NET side |

**Facts that decide the options**

- Sign-in: `muon` can take over a session from a signed-in device. The new
  client shows a code, the user enters it on the signed-in device, and
  `GET /auth/v4/sessions/forks/{selector}` returns the session with an
  optional opaque payload, which Proton's apps use to pass the key
  passphrase. Whether a fork made for a third-party client carries that
  payload is unverified. Proton's `pass-cli` uses a fork for its default
  `login`: it prints a URL, the user signs in in a browser (SSO and hardware
  keys included), and the CLI is signed in; `--interactive` asks for the
  password, TOTP and second password instead. `go-proton-api` signs in only
  with a username and password, plus 2FA and a human-verification token.
- Security model: every standalone route changes R2 and R3. protonctl would
  hold a session and the key passphrase (or, through `go-proton-api`, receive
  the password), and its own sockets would reach Proton's API. `muon` brings
  `reqwest` and `hyper`, which `deny.toml` bans today. Password handling was
  accepted on 2026-10-03 (Q4).
- Identity, tested 2026-10-03 without an account. The first request any
  client makes is an anonymous session (`POST /auth/v4/sessions`), and it
  checks the app version. `external-mail-protonctl@0.1.0-alpha` (also with
  `-stable`) and `external-calendar-protonctl@0.1.0-alpha` were refused with
  code 5002, "Invalid app version". `external-drive-protonctl@0.1.0-alpha`,
  Drive's documented third-party form, passed the version check and got
  8004, "Operation not allowed", for the anonymous session itself. Controls
  behaved: no header gave "Missing x-pm-appversion header", and a malformed
  name a format error. So Proton accepts an honest third-party identity for
  Drive only; a standalone Mail or Calendar client would have to pose as one
  of Proton's own apps, which Proton forbids. Proton can also gate a client
  per account: `pass-cli`'s login ends with a check that "your account is
  authorized to use the CLI". (`/api/tests/ping` answers without any app
  version, so it cannot test this.)
- Search: Proton's servers search message metadata; body search needs a local
  index, which Bridge keeps today (Proton's Rust equivalents are `mail-search`
  and `proton-foundation-search`).
- Stability: none of these libraries promises outsiders a stable interface.
  `muon` went from 1.7 (the fork in `clients`) to 4.1 (the registry).

**Options, cheapest first**

The identity test closes options 2 to 4 for Mail and Calendar unless Proton
grants protonctl a third-party identity; Bridge and the calendar link are
Proton's sanctioned paths. They stay listed for that case, and for Drive.

1. Keep Bridge, and make the Drive app optional: list and stat through the
   CLI when the folder is absent. **Built 2026-10-03**: without the folder,
   `list_folder`, `get_file_metadata`, `read_file_content` and
   `download_file` go through the CLI, and `search_files` says it needs the
   folder. An index built from Drive events would bring search back; not
   built.
2. Rust on `muon` and `proton-crypto-rs`, modelled on `pass-cli`: one
   language and one binary, with setup as a browser login. `muon` carries
   sign-in, human verification, session refresh and DNS fallback;
   protonctl would write the Mail, Calendar and Contacts calls and their
   decryption, with `go-proton-api` as the reference. `muon` and its
   `muon-proc` macros ship no license file, though Proton's own GPL projects
   build on them from the same public registry; asking Proton for a license
   file would close that gap.
3. A Go helper on `go-proton-api`: Mail, Calendar reads and writes, and
   Contacts, through Proton's most complete MIT-licensed library. Password
   sign-in is acceptable since Q4; the cost is a Go toolchain and a second
   binary beside the Rust one.
4. Rust on `proton-crypto-rs` (MIT) alone, with our own HTTP layer: one
   language and only MIT dependencies, but protonctl would own sign-in, 2FA,
   human verification, session refresh and every endpoint: the native client
   that [section 3](03-options.md) sized at 6k to 10k lines.

**Everything else**, classified from the listing; none is a Proton API client
or SDK:

- Reference only: apps that show how Proton's clients call the API but are not
  libraries: `WebClients`, `WebPackages`, `android-mail`, `android-calendar`,
  `ios-mail`, `ios-calendar`, `protoncore_android`, `protoncore_ios`,
  `ios-networking`, and in ProtonDriveApps `android-drive`, `ios-drive`,
  `mac-drive`, `windows-drive`, `windows-drive-block-verification`,
  `dotnet-security`, `sdk-tech-demo`.
- Unrelated crypto research, key transparency and demos (10): `cpp-openpgp`,
  `go-ecvrf`, `kt-auditor`, `pm-key-transparency-go-client`,
  `openpgp-interop-test-analyzer`, `openpgp-interop-test-docker`, `bip39`,
  `go-rfc5322`, `go-mime`, `proton-rust-nation-2026` (a Crux and Lumo demo).
- Infrastructure, server-side and web tooling (27): .github, android-fusion,
  apple-fusion, ct-monitor, get-random-values, go-netbox-dns,
  gomobile-build-tool, haproxy-health-check, interval-tree,
  k8s-proxy-image-swapper, libsieve-php, logging, mutex-browser, opendkim,
  php-coding-standard, primary-tab, proton-mobile-test, proton-parking,
  proton-telemetry, proton.rackndr, protonmail.github.io, pyprotonrebar,
  sieve.js, therecipe_env_darwin_arm64_513, therecipe_qt, vat-validation,
  x509-sign.
- Archived (35), mostly the 2016 to 2021 web apps and their tooling;
  `rust-mail` says it moved to `clients`: Illuminate-Foundation, account,
  componentGenerator, css-modules, design-system, encrypted-search,
  eslint-plugin-enforce-uint8array-arraybuffer, fe-proxy, go-appdir,
  inbox-desktop, key-transparency-web-client, pm-srp, pmcrypto,
  proton-account, proton-bundler, proton-calendar, proton-contacts,
  proton-drive, proton-i18n, proton-lint, proton-mail, proton-mail-android,
  proton-mail-settings, proton-pack, proton-shared, proton-translations,
  proton-version, pt-formgenerator, react-components, react-storybook,
  releaser, rust-mail, source-map-parser, u2f, webcrypto-spec.
- Forks of other projects (95): Android-Week-View, ApnsPHP, DOMPurify,
  Font-Awesome, GitHawk, GuzzleBundleRetryPlugin, Mellt, PHPMailer,
  PayPal-PHP-SDK, Squire, TNEFDecoder, TrustKit, WebAuthn,
  angular-gettext-cli, angular-gettext-tools, angular-toggle-switch,
  awesomplete, bcrypt, bitcoin-php, bootstrap, bootstrap-sass,
  buffertools-php, circl, conflux, docker-credential-helpers, emojionearea,
  go, go-autostart, go-imap, go-imap-quota, go-keychain, go-libfido2, go-mbox,
  go-message, go-mobile, go-singleinstance, go-smtp, go-vcard, gopsutil,
  html2text, htmlpurifier, ical.js, icofileloader, ios-receipt-parser,
  jQuery.Autosize.Input, jquery-ajaxchimp, jsmimeparser, libui, lightningcss,
  localsocket, malihu-custom-scrollbar-plugin, matomo-log-analytics,
  mimemessage.js, ng-pikaday, node-vcf, nsdate-helper, openpgpjs, panicwrap,
  paypal-merchant-sdk, pecl-mail-mailparse, php-adblock-parser,
  php-mime-mail-parser, php-mime-mail-parser-old, php-shunting-yard,
  php-u2flib-server, phpecc, predis, proxy-client-react, push.js, resty,
  rust-email_address, rustls-platform-verifier, scim-schema, sdk-core-php,
  seaweedfs, sieve-reference, smart-app-banner, systray, text-security,
  tidy-url, timezone-support, tobubus, ttag, u2f-ref-code, uap-core,
  uap-python, ui, ui-codemirror, uikit, validation, vcard, virtual-u2f,
  vobject, winhello, xgo.

---

[← Appendix A: Phase 0 findings](appendix-a-phase-0.md) · [Contents](../rfc-0001.md#contents) · [Appendix C: Role-play of aliases mode →](appendix-c-roleplay.md)
