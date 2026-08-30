# H/R/D/M in graffy: journal-mediated rating (adaptation v1 — DRAFT)

**Status:** Declared extension (MCW Constitution, Article V) of the anchored
H/R/D/M rubrics in
[mcw-framework `docs/experiments/hrdm_rubrics.md`](https://github.com/rainmana/mcw-framework/blob/main/docs/experiments/hrdm_rubrics.md).
Evidence: **L0** — instrument adaptation, unpiloted. DRAFT until ratified by
the framework author (@rainmana); samples recorded against a draft version
are calibration data only and are excluded from any headline series.

`rubric_version` written into every `HrdmSample`:
`mcw-hrdm@v0.2 + graffy-adaptation@v1-draft`

---

## Article V declaration block

- **Declaration.** This page extends the canonical anchored rubrics with a
  *rating-substrate mapping*: how the transcript-defined rating units
  (turn, exchange, window, repair episode, late discovery) are located in
  graffy **run journals and sessions**, plus a blinding procedure for
  journals and a dual-rater configuration in which the second rater is a
  model. The canonical definitions, the 0–3 scales, and the behavioral
  anchors are **unchanged and are not restated as new definitions** — where
  quoted below they are quotations, with the framework page as source of
  truth.
- **Non-contradiction.** Nothing here rescoreS or reinterprets an anchor.
  The mapping only says *where in a journal* the rater finds the things the
  anchors already name.
- **Falsification conditions.** (1) Inherited unchanged: quadratic-weighted
  κ < 0.4 on H and R after one calibration round across ≥ 12 rated units
  (both corpus floors met: ≥ 48 windows for H/D/M, ≥ 24 repair episodes for
  R) falsifies the instrument. (2) Adaptation-specific: if graffy's
  mechanical segmentation (§4) disagrees with a human rater's segmentation
  on more than 20% of episodes/windows in a calibration corpus, this
  *adaptation* fails as an instrument regardless of κ, and journal-mediated
  rating must not feed L3 claims until the mapping is repaired.
- **Layer 0 trace.** Same four questions anyone can ask — *how well do we
  understand each other? how hard was it to get back on track? how fast are
  we sliding apart? are we blaming the tool for a conversation problem?* —
  asked about a session you can replay.

---

## 1. Scope: what gets scored, and what does not

- **Scored: the session** — the Human ↔ AI coupling across a chain of
  runs. This is the canonical HCW–ACW scope. All four proxies apply.
- **Not scored in v1: intra-run node-to-node coordination** (draft ↔
  verify, facade subgraphs). Failure modes and IUs apply there under
  Article III substrate independence — graffy journals them — but H/R/D/M
  scoring of AI ↔ AI internals would be a *further* extension needing its
  own declaration, and **M is undefined there** (scope restriction:
  Human ↔ AI only). Journals of intra-run activity serve as *evidence* for
  session-level scores, not as separately scored interactions.

## 2. Rating-unit mapping (the load-bearing table)

| Canonical unit (transcripts) | In a graffy session (journals) |
|---|---|
| **Turn** | One party's contribution: the human's prompt / feedback / approval decision (journaled as intake IUs, `ApprovalRecord`s, retry feedback); or graffy's verified response for a run. |
| **Exchange** | One run on the conversation floor: prompt in → verified response out. |
| **Window** | Five consecutive session-linked runs, or one pre-declared task phase. H, D, M scored per window. |
| **Repair-initiating turn** | A retry request (human `--retry` feedback, or auto-retry carrying the judge's critique), or a human correction captured at intake (`IU_KIND_CORRECTION`). graffy *types* what a transcript rater must infer. |
| **Repair episode** | The span from a repair-initiating turn to the first run whose verdict passes **and** whose journal does not re-raise the same misalignment (no new FailureSignal implicating the same IUs). R scored per episode; R_ev = episodes per 10 exchanges. |
| **Dedicated repair turn** | A retry attempt inside an episode (a full journaled run whose purpose is repair rather than new work). |
| **Late discovery** | A FailureSignal or human correction whose `implicated_iu_ids` include an IU **introduced ≥ 3 exchanges earlier** (IU `run_id` linkage lets the rater point to both, mechanically). |
| **Capability-blame statement** | Human utterances only, as captured in prompts/feedback ("the model just can't do this"). graffy only sees what the human typed — windows without enough human text for the M anchors are marked **M-unratable**, never guessed. |
| *"Point to line numbers"* | Point to journal event `seq` numbers (stronger than line numbers: hash-addressed, replayable). A score that cannot cite `run_id:seq` is not a score. |

## 3. Anchors (canonical, quoted — source of truth is the framework page)

Scales are 0–3, anchored at every point. Summary of the anchor logic, for
rater convenience only — **rate from the framework page's full tables**:

- **H (per window):** H3 no misalignment, references used correctly; H2
  minor misalignment repaired in ≤ 2 dedicated repair turns, nothing
  redone; H1 rework/discard forced, or ≥ 2 failed clarification attempts on
  the same IU; H0 divergent goals pursued, wholesale rejection, or
  abandonment/reset.
- **R (per repair episode):** R0 ≤ 1 dedicated repair turn, nothing
  discarded; R1 two turns or minor rework; R2 3–5 turns or substantial
  redo; R3 > 5 turns, full restart, or repair abandoned with the
  misalignment standing. **R_ev** (count per 10 exchanges) reported
  alongside — many cheap repairs with low R is often a *good* sign.
- **D (per window, via the late-discovery measurement model):** D0 none
  (misalignment surfaces within 2 exchanges); D1 exactly one late
  discovery; D2 two, or one introduced > 10 exchanges back; D3 three or
  more, or end-of-window goal statements materially disagree.
- **M (per window, Human ↔ AI only):** M0 no capability blame — failures
  attributed to information not exchanged; M1 blame expressed once but
  withdrawn during repair; M2 blame recurs unverified, or
  capability-flavored corrective action (regenerate, switch model,
  dumb-down) while the needed IU was never externalized; M3 the strategy
  reorganizes around presumed incapability while the transcript shows the
  IU was never sent. The ground-truth caveat holds: unverifiable windows
  are **M-unratable**, recorded as absent (the proto's optional fields
  exist for exactly this).

## 4. What graffy pre-computes (assist, never replace)

From journals alone, graffy can propose — for the rater to confirm or
correct, with the disagreement rate feeding falsification condition (2):

- window segmentation (5-exchange boundaries or declared phases),
- candidate repair episodes (retry chains; CORRECTION-IU spans),
- candidate late discoveries (IU introduction → implication distance),
- R_ev counts (mechanical once episodes are confirmed).

The **anchored judgments themselves (H/R/D/M scores) are never
auto-computed** in this adaptation. A model may *rate* — as a rater, under
§5 — but graffy-the-harness does not score its own sessions.

## 5. Raters, blinding, and the dual-rater configuration

- **Human rater:** `graffy rate <session>` (shipped v1) walks the
  windows/episodes, shows the evidence, records 0–3 scores or unratable →
  `HrdmSample{source: HUMAN, rater_id, rubric_version}`.
- **Model rater (declared methodological extension):** a rater *graph*
  scores the same units against the same anchors →
  `HrdmSample{source: MODEL}`. Whether a model can rate coordination
  health reliably is itself an open empirical question — quadratic-weighted
  κ between the human and model raters is a reportable result, not an
  assumption.
- **Blinding:** journals leak condition information transcripts don't —
  `RoutingDecision.chosen_model/provider`, tier names, spec ids. The rating
  surface must present journals with those fields **redacted**
  (`--blinded`), or condition-blinded rating is impossible. Calibration,
  corpus floors, statistics, targets, adjudication: inherited unchanged
  from the framework page.

## 6. What this page does not claim

1. Not that these mappings are validated — L0 until the reliability
   protocol runs on real graffy sessions.
2. Not that mechanical segmentation is ground truth — it is a proposal the
   human confirms, with its own failure rule.
3. Not that graffy "implements MCW" or that journal-mediated rating
   "solves" measurement (Article VI) — graffy is a test bed that makes the
   canonical instrument cheaper to run and its evidence replayable.
4. Not that model raters are valid — that is a hypothesis this
   configuration exists to test.

---

*Ratification checklist for @rainmana: (a) unit mapping in §2 — especially
exchange=run and episode=retry-chain; (b) the M-unratable default in §3;
(c) the 20% segmentation-disagreement failure rule in the declaration
block; (d) the blinding field list in §5; (e) the rubric_version string.
Edit this file directly on GitHub — the edits are the ratification.*
