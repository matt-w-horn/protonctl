[RFC-0001](../rfc-0001.md) › 8. Review process

# 8. Review process

1. At the end of each phase, review the code against the threats of
   [section 5](05-security.md): update each threat's residual risk, and
   check that every tool maps to a policy row, no `Command` builds a shell
   string, stdout carries only MCP frames, and every IO path has a timeout.
2. `/code-review` on the Phase 1a changes and on each later phase.
   `/security-review` ran beside it on the Phase 2 changes (Q25); it was
   left out at first, before protonctl had cryptography of its own.
   Q25 also named Phase 3, which was withdrawn with aliases mode
   ([Q43](10-open-questions.md)).
3. Privacy: confirm by reading the code that each tool returns only the
   fields its schema lists, and that nothing writes content outside the
   download directory and the export folder.
4. Against the RFC: each claim the RFC makes about the code (sizes, counts,
   names, paths) is checked when its phase closes, as an earlier review
   did.

---

[← 7. Testing](07-testing.md) · [Contents](../rfc-0001.md#contents) · [9. Rollout and milestones →](09-rollout.md)
