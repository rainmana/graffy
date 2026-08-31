# Pinned H/R/D/M anchors (verbatim quotation)

Quoted VERBATIM from mcw-framework `docs/experiments/hrdm_rubrics.md`
(commit `8365d220`, SHA-256 `87ed2be62ef9233f2b6f98f573382a343eb8c343515033a10504532d366a54bb`).
Rendered in full by `graffy rate` — never paraphrased (P0.2). The framework
page is the source of truth; edits belong there, not here.

## Rating units and shared definitions

These definitions are used by every anchor below. Raters apply them
mechanically; disputes about them are instrument bugs to be filed, not
resolved ad hoc mid-rating.

| Term | Definition |
|---|---|
| **Turn** | One message by one party. |
| **Exchange** | An adjacent pair of turns (one from each party). |
| **Window** | Five consecutive exchanges (or one natural task phase, if the protocol pre-declares phase boundaries). H, D, and M are scored per window. |
| **Repair-initiating turn** | A turn whose primary function is repair: it contains a clarification request, a correction of the other party's stated understanding, a restatement request, an explicit re-grounding move ("let's step back…"), or an explicit synchronization move ("since last time, X changed"). |
| **Repair episode** | The span from a repair-initiating turn to the first turn where both parties proceed without re-raising that misalignment. R (cost) is scored per repair episode. |
| **Dedicated repair turn** | A turn inside a repair episode that advances repair rather than the task. |
| **Late discovery** | A misalignment surfaced three or more exchanges after the turn that introduced the diverging IU (the rater must be able to point to both turns). |
| **Capability-blame statement** | An utterance attributing a failure to the counterpart's intelligence, competence, or model quality ("it's just not smart enough," "the model can't do this"), as opposed to attributing it to information not exchanged. |

---

## H — MCW Health (0–3), per window

Canonical definition: *perceived shared understanding between participants*
(`0 = broken / 3 = strong`).

| Score | Behavioral anchor |
|---|---|
| **H3 — strong** | No misalignment surfaces in the window; both parties use references to earlier content correctly (no corrections needed); at most one trivial clarification, resolved within a single exchange. |
| **H2 — adequate** | Minor misalignment surfaces but is repaired within the window in ≤ 2 dedicated repair turns; no completed work is discarded or redone. |
| **H1 — strained** | At least one misalignment forces rework or discarding of a work product, OR the same IU requires ≥ 2 clarification attempts without convergence within the window. |
| **H0 — broken** | The parties demonstrably pursue different goals or referents; a work product is rejected wholesale; or the interaction is abandoned or reset within the window. |

---

## R — Repair Cost (0–3), per repair episode

Canonical definition: *effort required to realign after a coordination
failure* (`0 = low / 3 = high`).

| Score | Behavioral anchor |
|---|---|
| **R0 — low** | Realignment within ≤ 1 dedicated repair turn; no work discarded. |
| **R1** | Realignment within 2 dedicated repair turns, or minor rework (a small portion of produced content revised). |
| **R2** | Realignment required 3–5 dedicated repair turns, or a work product had to be substantially redone. |
| **R3 — high** | Realignment required > 5 dedicated repair turns, a full restart/reset, or repair was abandoned with the misalignment left standing. |

### R split: repair-event count (R\_ev) vs. repair cost (R)

The system-prompt derivation's Prediction 1 ("reduced early repair events")
needs a *count*, but canonical R is a *cost*. Conflating them makes the
prediction unscorable. This extension therefore splits:

- **R (canonical, unchanged):** the 0–3 ordinal cost per repair episode, as
  anchored above.
- **R\_ev (extension):** the number of distinct repair episodes initiated per
  10 exchanges. A count, not an ordinal; report it as a rate.

A healthy interaction can have high R\_ev with low R (many cheap repairs —
often a *good* sign), which is precisely the distinction the single letter
was erasing.

---

## D — Drift Rate (0–3), per window

Canonical definition: *speed at which the shared coordination state
diverges* (`0 = stable / 3 = rapid`).

**Measurement model (stated, not smuggled):** the shared coordination state
is not directly observable, so D cannot be rated from it. This proxy scores
the observable signature of drift — *late discoveries*: misalignments that
surface well after the turn that introduced them. Fast-surfacing
misalignment is a repair event, not drift; misalignment that incubates is
drift. This is a measurement model for canonical D, not a new definition.

| Score | Behavioral anchor |
|---|---|
| **D0 — stable** | No late discoveries; any misalignment surfaces within 2 exchanges of the turn that introduced it. |
| **D1** | Exactly one late discovery in the window. |
| **D2** | Two late discoveries, or one whose introducing turn lies more than 10 exchanges back. |
| **D3 — rapid** | Three or more late discoveries, or the parties' end-of-window statements of the current goal materially disagree (where the protocol elicits or the transcript contains such statements). |

---

## M — Misattribution (0–3), per window — **Human ↔ AI only**

Canonical definition: *tendency to blame coordination failures on agent
capability rather than shared context* (`0 = none / 3 = frequent`).

**Scope restriction (declared):** "agent capability" has no defined referent
when both parties are human. M is scored **only in Human ↔ AI interactions**.
The two Human ↔ Human toy experiments (1 and 5) do not score M; their
expected-signature tables predate this restriction and are reconciled in the
pre-registration pages. Defining an M analogue for Human ↔ Human settings
(e.g., blame directed at a partner's competence rather than at what was
never said) would be a further extension requiring its own declaration.

**Ground-truth caveat (stated, not hidden):** whether a failure "really" was
coordination rather than capability is exactly what the framework is trying
to establish (Limitation 4 of the paper outline). These anchors therefore
require *transcript evidence* — the rater must locate the needed IU and
verify it was never externalized — which is a checkable proxy for
coordination failure, not a resolution of the confound. Where the rater
cannot verify either way, the window is marked M-unratable rather than
guessed.

| Score | Behavioral anchor |
|---|---|
| **M0 — none** | No capability-blame statements; failures, where discussed, are attributed to information not exchanged. |
| **M1** | Capability blame is expressed once but withdrawn or corrected during repair ("ah — I never actually told you X"). |
| **M2** | Capability blame recurs (≥ 2 statements) without verification, or the human takes capability-flavored corrective action (regenerating, switching models, dumbing the task down) while the transcript shows the needed IU was never externalized. |
| **M3 — frequent** | The interaction strategy is reorganized around presumed incapability (wholesale distrust, abandonment, permanent oversimplification) while the transcript shows the needed IU was never externalized. |

---
