[RFC-0001](../rfc-0001.md) › 6. Privacy

# 6. Privacy

## Disclosure and retention

- **Flagged once:** whatever Claude reads leaves Proton's zero-access model
  and is processed by Anthropic under the Claude account's terms.
  Check the account's model-training setting before connecting mail.
- **Calendar link (decision Q1):** "When the URL is used,
  Proton Mail has access to this key to decrypt the calendar." Limits: a
  dedicated link, fetches at most every 15 minutes, Limited view (busy/free
  only) for any calendar whose details Claude does not need.
- Minimization: search returns metadata, plus 200-character text snippets
  only when a call asks for them; content only on explicit reads;
  exclusions apply before results exist.
- Retention: protonctl keeps its config. The hosts keep what protonctl
  returned in their transcripts and logs ([section 5](05-security.md)).
  The export folder, when one is configured, keeps what was exported until
  someone deletes it; choose a folder that no sync service mirrors unless
  that is wanted. The Phase 1b write log was withdrawn with Phase 1b;
  [RFC-0002](../rfc-0002.md) proposes one that holds IDs and no content
  (R34).
  Bridge's and the CLI's caches belong to Proton's clients. protonctl runs the
  CLI with `PROTON_DRIVE_LOG_LEVEL=ERROR`, which stops its DEBUG messages; it
  still writes metric events (counts, sizes, timings, its crypto model; no
  names or IDs) to `~/Library/Logs/proton-drive-cli`.
- No telemetry. protonctl's own sockets go only to 127.0.0.1.

## Privacy layer (withdrawn, Q43)

The privacy layer, which replaced names and IDs with aliases in aliases
mode, was withdrawn by [Q43](10-open-questions.md), because it let about
one name in nineteen through by design and was most of the code. The
host's model provider now sees results as Proton's apps show them, names
included, within the limits above: read-only tools, exclusions applied
before results exist, and hidden characters removed (R6).

---

[← 5. Security](05-security.md) · [Contents](../rfc-0001.md#contents) · [7. Testing →](07-testing.md)
