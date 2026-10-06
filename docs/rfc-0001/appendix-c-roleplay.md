[RFC-0001](../rfc-0001.md) › Appendix C: Role-play of aliases mode

# Appendix C: Role-play of aliases mode

Run 2026-10-04 to see whether Claude can tell the user who people are
("the lawyer who represents you in the lease") when every name is an
alias, with no help from the server beyond its instructions.

## Method

- Six runs, each a separate Claude session with no tools: three synthetic
  scenarios, each with two versions of the aliases-mode server
  instructions.
- **Base:** the aliases-mode instructions as drafted before this test,
  without the sentences on roles that the
  [API specification](lld-api.md#server-instructions) now holds. **Guideline:** the same, plus one sentence: call a person or
  organization by the role the results show, give the alias in
  parentheses the first time, say less rather than guess, and never
  guess a real name.
- Each run received the user's question and the tool results as aliases
  mode would return them once a name model runs (Phase 5), so organization
  names were aliases too, the harder case. It reported its reply, the
  evidence for every role it used, and what the aliases kept it from
  saying.
- Scenarios: a lease negotiation (the user, their lawyer, the other
  side's counsel, an insurance contact); a family week (a physio
  appointment, a parent-teacher conference, driving a parent to
  cardiology, a sibling's request); a senior engineer hire (two
  candidates, a colleague's debrief, an outside recruiter, CVs in Drive).

## Results

How each run referred to people:

| Scenario | Base | Guideline |
|---|---|---|
| Lease | "your lawyer (amber-falcon-river, Partner at velvet-stone-meadow)"; "counsel for tidal-oak-ember"; the insurance sender by alias only | "your lawyer (amber-falcon-river, a partner in the real estate practice at velvet-stone-meadow)"; "the other side's counsel (birch-lantern-cove)"; "an insurance contact (silver-wren-ledger)" |
| Family week | "the clinic", "the teacher", "your sister", "your mom", mostly without aliases | "the physio clinic (cedar-mist-anchor)", "the teacher (rowan-tide-sparrow)", "your sister (juniper-coast-ember)", "the cardiology office (auburn-heath-clinic)" |
| Hiring | "the external recruiter (fable-orchid-pike)"; "ash-river-lumen is the colleague who sent the debrief"; candidates by alias | "a colleague (ash-river-lumen)"; "an outside contact (fable-orchid-pike) who seems to be the recruiter"; "one candidate (coral-dune-thicket)" |

## Findings

1. **Claude describes people by role without being told.** All three base
   runs did it where the results supported a role. The guideline made the
   form consistent (the role first, the alias once, in parentheses) and
   kept thin evidence generic ("an insurance contact", "seems to be the
   recruiter").
2. **Where roles came from:** the `you`, `your-organization`, `external`,
   `education` and `webmail` hints; organizer and attendee fields; event
   titles ("Physio – knee follow-up"); and what people wrote about
   themselves (a signature's "Partner, Real Estate Practice", "Love, your
   sister"). The last kind is written by the sender, who can claim any
   role; the base runs noted this among their limits but not in their
   replies.
3. **No run guessed a real name, used a `reveal_*` tool, or read meaning
   into an alias's words.** Two runs noticed that some test aliases
   contained telling words ("clinic", "school", "legal") and set them
   aside explicitly.
4. **What the aliases cost:** a link (a URL alias) the user had to open
   in Proton instead; places that could not be used to plan travel; an
   organization behind an email domain that stayed unknown; and, in the
   lease, whether the user was landlord or tenant. Matching refs across
   calls is what let a run connect a CV in Drive with its sender in mail.
5. **Not caused by aliases:** the family scenario gave two events weekdays
   that did not match their dates. The base run caught both mismatches;
   the guideline run repeated one. With one run per cell this says
   nothing about the guideline.

## What changed

- The aliases-mode instructions gain the guideline, with one more clause
  from finding 2: when a role rests only on what that person wrote about
  themselves, say so if it matters
  ([API specification](lld-api.md#server-instructions)).
- The word list's curation drops words that name a kind of place,
  organization, role or relation ("clinic", "school", "legal", "bank",
  "mother"), from finding 3 ([section 6](06-privacy.md#identifiers), Q18).
- The README demo shows Claude naming a person by role, with the alias in
  parentheses.
- The server stays as designed: describing people is Claude's work, from
  the hints, fields and text a result already holds.

Limits of this test: one run per cell, synthetic data written for the
test, and Phase 5 tokenization assumed. M2.10's live check repeats it on
real results in aliases mode; that repeat has not run
([#18](https://github.com/matt-w-horn/protonctl/issues/18)).

## First real results (2026-10-04)

Before that repeat, the maintainer read one real document from Drive in
aliases mode and compared it with the original. Short forms of names,
initials and a company that never sent mail came back raw; one person had
two aliases; employers and services that send mail came back as person
aliases, which hides what they are; and project names were not touched.
OCR errors and misspellings are expected to pass too. These are defects
D1 to D7 in [section 9](09-rollout.md#phase-2-defects-found-on-real-results),
with the evaluation (D1) to measure them.

---

[← Appendix B: Proton's open-source code](appendix-b-proton-code.md) · [Contents](../rfc-0001.md#contents) · [Low-level design →](lld-privacy-layer.md)
