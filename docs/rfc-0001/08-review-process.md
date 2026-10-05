[RFC-0001](../rfc-0001.md) › 8. Review process

# 8. Review process

1. At the end of each phase, re-run the
   [security and privacy review](security-privacy-review.md) against the
   code: update each invariant's status and each threat's residual risk,
   and check that every tool maps to a policy row, no `Command` builds a shell string, stdout
   carries only MCP frames, every IO path has a timeout.
2. `/code-review` on the Phase 1a changes and on each privacy phase.
   `/security-review` runs beside it on the Phase 2 and Phase 3 changes,
   which add protonctl's own cryptography and user presence (Q25); it was
   left out on 2026-10-02, before either existed.
3. Privacy: confirm by reading the code that each tool returns only the
   fields its schema lists; that in aliases mode every result leaves
   through the privacy pipeline (R13), errors and page tokens included, and
   nothing writes content to disk (R10); and that in off mode nothing
   writes content outside the download directory and the export folder.
4. Against the RFC: each claim the RFC makes about the code (sizes, counts,
   names, paths) is checked when its phase closes, as the review of
   2026-10-04 did (commit 7786b3e).

---

[← 7. Testing](07-testing.md) · [Contents](../rfc-0001.md#contents) · [9. Rollout and milestones →](09-rollout.md)
