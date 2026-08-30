# H/R/D/M in Graffy: journal-mediated rating

**Status:** Article V adaptation proposal - author-review draft, **not ratified for study or headline use; calibration-only collection is permitted**.

**Evidence layer:** **L0 - instrument design, unpiloted.** This document specifies a mapping and an implementation conformance test. It does not validate H/R/D/M, Graffy, model judges, automatic repair, or the MCW Framework.

**Canonical sources pinned for this review:**

- MCW Framework repository commit `8365d220f2676f248c934e20f23e427e01cf3ce8`
- `docs/experiments/hrdm_rubrics.md` SHA-256 `87ed2be62ef9233f2b6f98f573382a343eb8c343515033a10504532d366a54bb`
- MCW Constitution v1.1, `docs/constitution.md` SHA-256 `14bdfa2702d882d2786447625bf1b42b34deb6ff7cee7e8d99bc0ce004c2450a`

**Implementation reviewed:** Graffy `fa3132b5b332f58e92b98ce662b60efd0e5489d4`.

Existing samples stamped `mcw-hrdm@v0.2 + graffy-adaptation@v1-draft` are calibration artifacts only. They are not headline-eligible H/R/D/M data and must not be silently relabeled after this document changes.

Recommended draft identifier for new calibration samples:

`mcw-hrdm@8365d220+sha256.87ed2be6 + graffy-journal-map@v1-draft`

The `-draft` suffix is removed only after the conformance gates in section 8 pass and the framework author explicitly ratifies that exact version.

---

## Article V declaration block

- **Declaration.** This page proposes a rating-substrate mapping from the canonical transcript units - turn, exchange, window, repair episode, dedicated repair turn, late discovery, and capability-blame statement - to an **external-boundary transcript projection** derived from Graffy journals. It also proposes an adaptation-specific segmentation check and a condition-blinded journal rating procedure.
- **Non-contradiction.** A Graffy run, node, model call, retry attempt, or journal file is an implementation container, not automatically a canonical turn or exchange. Canonical H/R/D/M is scored only over observable Human-to-AI and AI-to-Human boundary contributions. Internal node-to-node and automatic retry activity is reported separately and is not silently relabeled as canonical H/R/D/M.
- **Falsification conditions.** The adaptation inherits the canonical reliability protocol unchanged: at least 12 transcripts/sessions independently rated by at least two raters, at least 48 rated windows for H/D/M, at least 24 rated repair episodes for R, and the canonical kappa/alpha targets and failure rules. In addition, this journal adaptation fails if, after one calibration round, its mechanical boundary projection or segmentation disagrees with a human segmentation on more than 20% of eligible units. The denominator, disagreement rules, exclusions, and adjudication must be frozen before calibration. Failure of this adaptation blocks L3 claims from journal-mediated ratings even if score agreement is otherwise acceptable.
- **Layer 0 trace.** Graffy must first reconstruct the conversation the human and AI actually had. Only then can a rater ask: *How well did they understand each other? How hard was it to get back on track? How late did divergence surface? Did the human blame capability for information that was never exchanged?*

---

## 1. Scope and separation of observational layers

### 1.1 Canonical rating scope

Canonical H/R/D/M applies to the external HCW-ACW coupling across a session. The rating corpus is the ordered sequence of contributions that crossed the human/AI boundary.

The following can become boundary turns when their content actually crosses that boundary:

- the human's initial prompt;
- subsequent human corrections, clarifications, edits, approval decisions, or feedback;
- a verified AI response shown to the human;
- an AI clarification or repair request shown to the human.

### 1.2 Internal orchestration scope

Draft nodes, judge outputs, tool calls, routing decisions, automatic critiques, hidden retry attempts, and other node-to-node activity are internal orchestration telemetry. IUs and the six failure modes can apply to this coordination under Constitution Article III. Canonical MCW remains scoped to HCW-ACW coupling, and canonical H/R/D/M is not assigned to AI-to-AI internals by this adaptation.

Internal telemetry can support separate Graffy-native measures such as:

- attempts to produce the first verified response;
- internal failure signals by mode;
- automatic-repair token, time, turn, and monetary cost;
- internal convergence or retry-budget exhaustion;
- escalation frequency and outcome.

These values are useful implementation observables. They are not canonical H, R, D, or M unless a later Article V extension defines and validates that mapping.

### 1.3 Current implementation consequence

At Graffy commit `fa3132b`, a session is created for one prompt and reused across its automatic retry attempts. The CLI does not yet expose continuation of the same session across multiple human prompts. Therefore:

- a retry session generally represents **one attempted external exchange**, not several exchanges;
- failed attempts without a verified response are not completed exchanges;
- five retry attempts cannot be treated as a canonical five-exchange window;
- the current `graffy rate` implementation cannot yet construct a canonical five-exchange H/D/M window from ordinary `graffy run --retry ...` output.

Until boundary-session continuation exists, retry sessions are eligible for internal convergence analysis only. They are not eligible for canonical H/D/M scoring.

---

## 2. Boundary transcript projection

Graffy must derive a condition-blind sequence of `BoundaryTurn` records before proposing rating units. Each record needs:

- `actor`: human or AI;
- verbatim boundary-visible content;
- timestamp and stable order;
- one or more immutable journal references in `journal://<run_id>/<seq>` form;
- boundary role, such as prompt, correction, clarification, approval edit, repair request, or verified response;
- visibility status proving that the content reached the counterpart;
- provenance sufficient to detect deduplication across automatic attempts.

Internal drafts, hidden judge critiques, tool results not shown to the human, automatic retry feedback, routing events, and failed attempts without a visible AI response are excluded from the canonical boundary projection.

When the same initial prompt is copied into multiple automatic attempts, it remains one human boundary turn. When several attempts culminate in one verified response, the attempts collectively produce one AI boundary turn.

If Graffy cannot prove whether content crossed the boundary, the projection marks it unknown and the affected unit is unratable. It does not guess.

---

## 3. Rating-unit mapping

| Canonical unit | Graffy journal mapping |
|---|---|
| **Turn** | One `BoundaryTurn`: a single contribution by one participant that actually crossed the HCW/ACW boundary. A run can contain zero, one, or multiple boundary turns. |
| **Exchange** | An adjacent Human-to-AI or AI-to-Human pair of boundary turns, following the canonical definition. A standard successful one-prompt/one-response run often yields one exchange, but this is a checked special case rather than an identity. |
| **Window** | Five consecutive completed boundary exchanges, or one natural task phase whose boundaries were declared before rating. An incomplete trailing group is not automatically scored as a window. |
| **Repair-initiating turn** | A boundary turn whose primary function is clarification, correction, restatement, re-grounding, or synchronization. A hidden judge critique or automatic retry is not a canonical repair-initiating turn. |
| **Repair episode** | From an external repair-initiating boundary turn to the first boundary turn after which both parties proceed without re-raising that same misalignment. Closure requires IU/failure continuity evidence, not merely a successful run status. |
| **Dedicated repair turn** | A boundary turn inside a repair episode that advances repair rather than the task. Hidden automatic attempts do not count. |
| **Internal repair sequence** | Graffy extension: a bounded automatic retry chain from an internal failure signal through convergence or exhaustion. Report separately from canonical R. |
| **Late discovery** | A misalignment surfaced at least three completed boundary exchanges after the boundary turn that introduced the diverging distinction. Both introduction and discovery require journal citations. Internal attempt count is irrelevant. |
| **Capability-blame statement** | A human boundary utterance attributing failure to counterpart intelligence, competence, or model quality. Capability-flavored human actions can count only when they are observable and attributable to the human. Automatic routing does not count. |
| **Rating citation** | One or more `journal://<run_id>/<seq>` references. A score requiring evidence that cites no underlying events is not a valid score. |

---

## 4. Canonical anchors and the M distinction

The full anchor tables in the pinned canonical `hrdm_rubrics.md` remain authoritative. Graffy must display those anchors during rating rather than replacing them with shortened labels.

- **H, per five-exchange window:** 3 means strong shared understanding and 0 means broken coordination, using the canonical behavioral anchors.
- **R, per external repair episode:** 0 means low repair cost and 3 means high repair cost, using dedicated **boundary** repair turns and observable rework.
- **D, per five-exchange window:** scored through canonical late-discovery evidence measured in completed boundary exchanges.
- **M, per five-exchange Human-AI window:** scored from human capability-blame and evidence about whether the needed IU was externalized.

The distinction between **M0** and **M-unratable** is binding:

- A complete boundary transcript with no capability-blame statements is **M0**.
- A window is **M-unratable** when the record is incomplete or the rater cannot verify the contextual evidence required to distinguish capability failure from unexternalized information.
- Absence of capability blame is not itself a reason to mark M unratable.
- The rater must not infer untyped human thoughts, motives, or actions.

Any current UI instruction saying "use `u` if no capability-blame evidence" must be replaced with this distinction.

---

## 5. Segmentation, scoring, citations, and blinding

### 5.1 Mechanical assistance

Graffy can propose:

- boundary-turn extraction;
- adjacent-pair exchange segmentation;
- complete five-exchange windows;
- candidate external repair episodes;
- candidate late discoveries;
- internal retry sequences as a separate telemetry class;
- canonical R_ev after external repair episodes are confirmed.

The human rater confirms or corrects every proposed segment. Mechanical proposals never become scores by themselves.

### 5.2 Required rating surface

For each unit, the rating interface must show the complete relevant boundary text and stable journal references. It must collect:

- a 0-3 score or unratable;
- an unratable reason when applicable;
- required evidence references for every score whose anchor depends on an observed event;
- segmentation confirmation or correction;
- rater identity and rater type;
- rubric identifier, adaptation identifier, and blinding status.

Showing only run IDs, status, machine-assigned failure labels, and repair counts is insufficient for canonical rating.

### 5.3 Blinding

Condition-blinded rating must redact or neutralize fields that reveal the experimental arm or prime the requested judgment, including:

- provider, model, capability tier, and routing reason;
- graph/spec identifiers, names, tags, and hypothesis-bearing labels;
- system prompts and manipulation notes;
- machine-assigned failure-mode and repair-operation labels;
- hidden retry counts, hidden judge verdicts, and internal confidence values;
- expected-signature tables and condition names.

The boundary conversation content and its order remain visible. A `--blinded` flag is not enough by itself; the exported projection and redaction rules must be tested.

### 5.4 Independent raters and reliability

At least two raters independently score the same frozen projections while blinded to condition and each other's scores. Graffy must preserve both raw rating streams. Consensus scores can be exploratory but never replace the independent scores in reliability reporting.

Report quadratic-weighted Cohen's kappa per proxy and ordinal Krippendorff's alpha under the canonical protocol. No sample becomes L3 merely because the framework author supplied one rating.

---

## 6. Score provenance and schema requirements

`HrdmSample` needs enough information to reproduce the judgment. At minimum it must preserve:

- canonical rubric content pin;
- Graffy adaptation version;
- `source = HUMAN_RATER` for analyst ratings;
- stable unit ID and exact boundary range;
- cited `journal://<run_id>/<seq>` evidence references;
- blinding profile/version;
- segmentation algorithm/version;
- optional unratable reason per proxy;
- rater identity or pseudonymous rater ID;
- calibration versus study status.

`HUMAN_SURVEY` is not the correct provenance label for an analyst applying a rubric. Preserve the existing enum value for wire compatibility and add a distinct `HUMAN_RATER` value.

The canonical rubric pin must identify immutable content. `mcw-hrdm@v0.2` alone is insufficient because it names the framework release while the rubric file can change. Use a commit and/or content hash.

---

## 7. Adaptation-specific segmentation failure rule

The proposed 20% rule is accepted in principle as an Article V falsification condition, subject to this frozen operationalization:

1. The calibration corpus includes at least 12 complete sessions and contains both ordinary exchanges and external repair episodes.
2. Before rating, one human annotator independently marks boundary turns, exchanges, complete windows, repair-episode starts/ends, and late-discovery pairs.
3. A unit disagrees when the mechanical proposal changes its inclusion, actor, boundary visibility, start, end, or canonical class after human review.
4. Report disagreement separately for turns, exchanges/windows, and repair episodes; do not hide a failed class inside a pooled average.
5. If disagreement exceeds 20% for any load-bearing class, the adaptation fails and must be revised before journal-mediated ratings support L3 claims.
6. Human corrections remain in the audit trail. They do not silently overwrite the mechanical proposal.

This rule is additional to the canonical rule that more than 20% unratable windows is an instrument-failure signal. Segmentation disagreement and unratability are different quantities and both must be reported.

---

## 8. Ratification and implementation conformance gates

This adaptation remains draft until all gates pass:

- [x] Boundary turns are reconstructed from full journal events rather than run summaries.
- [x] Duplicate prompts across automatic attempts are collapsed into one human boundary turn.
- [x] Failed attempts without a visible verified response are not counted as exchanges.
- [x] Automatic retry sequences are separated from canonical external repair episodes.
- [x] Sessions can contain at least five real external exchanges or a pre-declared natural phase.
- [x] Incomplete trailing groups are not silently scored as five-exchange windows.
- [x] M0 and M-unratable follow section 4.
- [x] The interface displays canonical anchors and underlying boundary evidence.
- [x] Required journal citations and unratable reasons are stored with samples.
- [x] Blinded projections are implemented and tested.
- [x] Multiple external repair episodes in one session are representable.
- [x] R_ev is computed only from confirmed external repair episodes.
- [x] Human-rater provenance is distinct from participant survey provenance.
- [x] Rubric/adaptation content is immutably pinned.
- [ ] Independent rating streams and reliability statistics are supported.
- [x] Existing draft samples remain draft calibration data.
- [ ] The framework author explicitly ratifies the exact conforming version.

Ratification is an author decision for this Graffy adaptation because this document chose that governance gate. Constitution Article V itself specifies declaration, non-contradiction, falsifiability, and traceability; it does not state that only the original framework author can create or ratify every extension. Constitution amendments are separately restricted by the Amendment Procedure.

---

## 9. Author review of Fable's five questions

### 1. Exchange = one run; window = five runs?

**Decision: No as a general identity; conditionally acceptable as a checked special case.**

One successful run can represent one exchange only when it contains exactly one external human contribution followed by exactly one external AI contribution. Runs with approval/edit turns can contain more. Failed and hidden retry runs can contain fewer. Windows must contain five completed boundary exchanges, not five journal files or attempts.

### 2. Repair episode = retry chain from first failed run through first pass?

**Decision: No for canonical R.**

An automatic retry chain is an internal repair sequence and a valuable Graffy metric. A canonical repair episode begins with an external repair-initiating turn and closes when the same misalignment no longer reappears as both participants proceed. A passing run alone does not prove that causal closure.

### 3. No typed evidence for M means unratable?

**Decision: No.**

A complete transcript with no capability-blame statements is M0. Unratable is reserved for incomplete or insufficient evidence about the capability-versus-context attribution required by the anchors.

### 4. More than 20% mechanical segmentation disagreement falsifies the adaptation?

**Decision: Yes in principle, with the operational definition in section 7.**

The rule must use predeclared denominators and must report disagreement by unit class. It supplements rather than replaces the canonical reliability and unratability rules.

### 5. Version string `mcw-hrdm@v0.2 + graffy-adaptation@v1`?

**Decision: No in its current form.**

Pin the canonical rubric by immutable commit/content hash and retain `-draft` until implementation conformance and explicit author ratification. Recommended draft identifier:

`mcw-hrdm@8365d220+sha256.87ed2be6 + graffy-journal-map@v1-draft`

### Ratification verdict

**Do not ratify the current implementation or remove `-draft`.** The conceptual adaptation is promising and repairable. Sections 1-8 define the version that can be ratified after implementation catches up.

---

## 10. Additional implementation and measurement findings

These findings are appended so the ratification review and the implementation repair plan remain one artifact.

### Blocker A - MCW evidence layers are redefined and conflated with source quality

The MCW Constitution defines five study-level evidence layers for empirical claims about MCW dynamics:

- L0 Illustration
- L1 Practitioner observation
- L2 Designed pilot
- L3 Pilot with reliability
- L4 Controlled study

Graffy's protobuf instead defines L0 Definitional, L1 Observational, L2 Empirical, and L3 Validated, with no L4. It then assigns those values to ordinary artifacts such as prompts, model outputs, and MCP results. This is not the canonical Article IV ladder and risks making, for example, an MCP result look like designed-pilot evidence.

**Required correction:** separate two systems:

1. an operational claim/source-support schema for documents, human input, model inference, tool output, provenance, and claim-to-source lineage; and
2. the canonical `McwEvidenceLayer` L0-L4 used only for empirical claims about MCW dynamics.

Do not call the first system the MCW Article IV evidence ladder.

### Blocker B - the declared evidence floor is not mechanically enforced

At `fa3132b`, `VerifyNode` places `policy.evidence.min_level` in the judge prompt but does not compare the draft's actual support level or cited artifacts against the policy. A model-inference draft recorded at L0 can pass a nominal L1 floor whenever the judge returns `PASS`.

**Required correction:** enforce the floor deterministically before model judgment. Reject or route unsupported claims when their linked support cannot meet policy. The model judge can assess relevance and entailment, but it must not override a failed structural floor.

### Blocker C - evidence exists beside claims without claim-level lineage

When a model draft consumes an MCP/tool IU, the resulting draft currently receives a new model-inference evidence ID but does not inherit or cite the source IU's evidence IDs. The journal can contain the tool receipt while the final claim has no lineage to it.

**Required correction:** propagate source-IU and artifact lineage into derived claims, record which source supports which claim or distinction, and test the final response's evidence closure rather than merely testing that some artifact exists somewhere in the run.

### Blocker D - fabricated five-stage fidelity records

`five_stage_records()` emits Selection, Encoding, Transmission, Decoding, and Integration for every IU with `fidelity_estimate = 1.0`, including latent human stages Graffy cannot observe. All five timestamps are generated together. This records perfect meaning survival rather than unknown measurement and can contaminate every future MCW dataset.

**Required correction:** make fidelity optional; record only stages that are actually observed or explicitly inferred; attach measurement source/method and uncertainty; never encode unknown as 1.0 or 0.0. Human Selection, Encoding, Decoding, and Integration generally remain unobserved unless a study provides a defensible instrument.

### Blocker E - hard-coded salience values look observational

Intake, draft, review, and response IUs receive fixed salience values such as 1.0, 0.8, and 0.9. These are implementation priors based on node role, not measurements of shared coordination salience.

**Required correction:** rename them as assigned policy weights with provenance, or make observational salience optional and unknown by default. Do not mix routing priority with measured MCW salience.

### Blocker F - model-judge failure labels are hypotheses, not detected facts

On `REVISE`, a model judge names one failure mode and Graffy records it as `FailureSignal` with a fixed confidence of 0.6. The signal has no linked evidence artifact, the confidence is uncalibrated, and the schema comment describes `early_signal` as journaled evidence rather than inference.

**Required correction:** label these as model-rater hypotheses, attach the judge-output artifact, retain `UNSPECIFIED` without force-fitting, replace fixed confidence with absent/uncalibrated unless a calibration study supports it, and keep human-confirmed and heuristic detections distinct.

### Blocker G - repair success is inferred from run success

Automatic repair actions are marked successful when the later run succeeds. Canonical repair closure requires evidence that the same implicated misalignment stopped recurring. Overall run success does not establish that causal link.

**Required correction:** identify the target failure/IUs, test whether that failure reappears, and distinguish `attempt_completed`, `run_passed`, and `target_failure_resolved`. Report causal efficacy only from the last field.

### Blocker H - the current rating UI cannot support canonical ratings

The current `graffy rate` path folds journals into `RunMetrics` and displays run status, failure labels, and repair counts. It does not display the complete boundary transcript, collect required journal citations, capture unratable reasons, implement blinding, or preserve segmentation corrections. It also proposes at most one repair episode and includes a final partial window.

**Required correction:** implement sections 2, 5, 6, and 8 before collecting non-calibration H/R/D/M samples.

### Blocker I - rater provenance is mislabeled

The CLI stores rubric judgments as `SCORE_SOURCE_HUMAN_SURVEY`. An analyst rating a transcript is not a participant completing a survey.

**Required correction:** add `HUMAN_RATER`; preserve `HUMAN_SURVEY` for direct participant self-report instruments if those are later designed.

### Blocker J - IU granularity limits D and repair-link claims

The intake node currently stores an entire prompt as one Goal IU. The MCW Framework explicitly leaves IU individuation open. This coarse unit can preserve a verbatim record, but it cannot by itself support fine-grained claims about which distinction drifted, which constraint was repaired, or when a specific IU was introduced.

**Required correction:** keep the verbatim parent IU, add explicit child distinctions only through a declared segmentation method, preserve parent-child lineage, and validate segmentation before using IU counts or fine-grained late-discovery metrics.

### Binary validation note - alpha.5

The supplied `graffy-v0.1.0-alpha.5-x86_64-unknown-linux-musl` binary was smoke-tested independently of the source review. It is a stripped, statically linked x86-64 PIE and reported version `0.1.0-alpha.5`. Store initialization, built-in graph discovery, an offline `intake -> ground -> draft -> verify -> respond` run, replay, and JSON metrics completed successfully. The smoke run emitted 22 events, four IUs, two evidence artifacts, two model-call records, and a successful terminal status.

This binary predates the `fa3132b` rating implementation, so it validates the earlier execution/journal substrate but not `graffy rate`. Its JSON output independently confirmed the operational labels `l0_definitional` and `l1_observational` discussed in Blocker A.

**Release-hygiene note:** the supplied tar archives preserve numeric builder ownership `1001:1001`. Privileged/container extraction can fail while attempting to restore that ownership unless `--no-same-owner` is used. Package release archives with normalized owner/group metadata. This does not affect MCW ratification.

---

## 11. What this document does not claim

1. It does not claim that Graffy implements MCW. Graffy is a test bed and coordination-instrumentation system.
2. It does not validate the canonical H/R/D/M rubrics or this journal adaptation.
3. It does not treat internal retry convergence as evidence of repaired Human-AI coordination.
4. It does not treat model-judge labels, hard-coded salience, or fabricated fidelity as observations.
5. It does not authorize existing draft samples for headline analysis.
6. It does not make the framework author's ratings sufficient for inter-rater reliability.
7. It does not prevent exploratory collection, provided every output remains explicitly labeled calibration-only and draft.

---

## 12. Post-fix ratification checklist for the framework author

After the implementation gates pass, ratification should explicitly answer:

1. Does the boundary projection preserve canonical turns and exchanges?
2. Are internal retry sequences excluded from canonical R while retained as separate telemetry?
3. Does the implementation distinguish M0 from M-unratable correctly?
4. Are complete five-exchange windows and external repair episodes segmented reproducibly?
5. Do rating samples contain immutable source pins, evidence citations, rater provenance, blinding status, and calibration/study status?
6. Have the 20% segmentation-disagreement rule and denominators been frozen before calibration?
7. Have evidence-layer, fidelity, salience, failure-confidence, and repair-closure contamination paths been removed?
8. Is the exact adaptation version being ratified immutable and represented by the emitted `rubric_version`?

Ratification must name the exact Graffy commit and document hash. Later semantic or implementation changes require a new adaptation version and a new calibration decision.
