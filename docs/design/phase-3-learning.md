# Phase 3 design: the MCW learning loop & research instrument

> Status: ACTIVE. C1 (detectors) and C5 v1 (`graffy metrics`) are shipped;
> this doc is the map for the rest. It also serves as the **MCW coverage
> audit**: what of the framework is implemented, what remains, and what data
> graffy can put in front of other researchers. Companion reading:
> `AGENT.md` (how to work on this repo), `docs/design/phase-2-mcp.md`
> (the facade/taxonomy design), and the framework itself:
> <https://rainmana.github.io/mcw-framework/> (W. Alec Akin, Apache-2.0).

## 1. MCW coverage matrix (honest audit, 2026-09-04)

Legend — **Schema**: typed in `src/protos/mcw.proto`. **Journaled**: emitted
at runtime into run journals. **Acted on**: changes execution behavior.
**Measured**: aggregated by `graffy metrics`.

| MCW construct | Schema | Journaled | Acted on | Measured |
| --- | --- | --- | --- | --- |
| Information Units (typed, salience, provenance) | ✅ | ✅ every run | ✅ ledger feeds nodes | ✅ counts |
| IU five-stage lifecycle (selection→integration) | ✅ `IuStageRecord` | ⚠️ stages recorded only where nodes set them | ⬜ | ⬜ per-stage fidelity |
| Failure mode: Drift | ✅ | ✅ judge-named (C1) | ✅ C2 feeds the critique back as repair | ✅ frequency |
| Failure mode: Asymmetric State Advancement | ✅ | ✅ judge-named (C1) | ✅ **prevented** — effector approval gates | ✅ frequency |
| Failure mode: False Alignment | ✅ | ✅ judge-named (C1) + interview contradiction probe | ⚠️ interview: conservative annotation wins | ✅ frequency |
| Failure mode: Overcompression | ✅ | ✅ judge-named (C1) | ⚠️ digest nodes carry preserve-distinctions knowledge | ✅ frequency |
| Failure mode: Constraint Opacity | ✅ | ✅ judge-named (C1) | ✅ tool-shape drift warning at run start | ✅ frequency |
| Failure mode: Repair Suppression | ✅ | ✅ judge-named (C1) | ⬜ | ✅ frequency |
| Convergence exhaustion (not a canonical mode — kept `Unspecified`, never force-fit) | ✅ | ✅ visit-cap signal (C1) | ✅ halts the path | ✅ cap-hit runs |
| Repair op: Re-grounding | ✅ node kind | ✅ retry attempts journal RepairActions (C2) | ✅ `--retry` wires detection→repair | ✅ counts+token cost |
| Repair op: Decompression | ✅ node kind | ✅ via C2 when the judge names overcompression | ✅ | ✅ |
| Repair op: Re-weighting | ✅ node kind | ⚠️ no path emits yet (no designated mode in canon) | ⬜ | ✅ (ready) |
| Repair op: Disambiguation | ✅ node kind | ✅ via C2 when the judge names false_alignment | ✅ `PauseForDisambiguation` + interview follow-ups | ✅ |
| Repair op: Synchronization | ✅ node kind | ✅ via C2 when the judge names asymmetric_state_advancement | ✅ | ✅ |
| H/R/D/M observables | ✅ `HrdmSample` (anchored ordinal scores + rubric version + rater) | ✅ human calibration samples via `graffy rate` | ⬜ no adaptive behavior; model rater not built | ✅ sample and rating-journal counts |
| Operational support ladder | ✅ `SupportLevel` | ✅ every artifact | ✅ deterministic support floor + strict/trace-only policy | ✅ distribution |
| MCW Article IV evidence layers L0–L4 | ✅ `McwEvidenceLayer` | ⬜ no empirical-study path assigns them | ⬜ | ⬜ |
| Evidence artifacts (hash-addressed) | ✅ | ✅ | ✅ ground-before-draft floor | ✅ counts |
| McwStateSnapshot | ✅ | ⬜ | ⬜ | ⬜ |

**Summary for the impatient**: the *observability* layer of MCW is in and
measured (IUs, six failure modes via judge detection, evidence levels,
convergence). The *repair* layer is now
detection-driven (C2): failed attempts re-run with the judge's critique as
CORRECTION IUs and journal RepairActions with honest observed costs. The *H/R/D/M scoring* layer has its
canonical anchors in the framework repo; graffy's journal adaptation and
human calibration collection path are implemented, but the adaptation still
awaits ratification (§4). Detection today is judge-based (one detector); per-mode
heuristic detectors are future work that the journal format already supports.

## 2. What ships the learning loop (C2–C4)

- **C2 — retry with feedback** (`graffy run --retry n|auto`) — **SHIPPED v1**: when a run
  ends REVISE-exhausted or failed, re-run it in the same session with the
  judge's critique + named failure mode injected as CORRECTION IUs. Budgeted
  always (attempt cap, token/USD/second ceilings); `auto` stops on PASS,
  budget exhaustion, or no-improvement. Each attempt is a full journaled run
  linked by session; the convergence series (attempts → verdicts) is itself
  research data. Named mode → repair mapping: drift/overcompression →
  re-grounding + decompression knowledge in the retry prompt; false
  alignment → disambiguation question surfaced; constraint opacity →
  constraints restated explicitly. RepairAction events finally get emitted
  by a real path (with `triggered_by_failure_id` back-links and token costs).
  V1 scope: stops on PASS, attempt cap, or a missing signal. The open issue
  still describes no-improvement early stopping and broader human-in-the-loop
  repair work beyond the shipped slice.
- **C3 — feedback meta-eval**: optional judge-the-judge pass scoring the
  critique's specificity and actionability before it is trusted as feedback;
  doubles as the **model-rater path for HRDM sampling** (§4).
- **C4 — durable Lessons**: distill resolved failure→repair pairs into a
  lessons store (libSQL), injected two ways: agent-facing (knowledge added
  to matching future graphs) and human-facing (`graffy lessons` — plain
  language prompting advice citing the MCW construct, e.g. "your skill
  prompts under-specify constraints → Constraint Opacity × 7 this month").

## 3. The metrics catalog

**Shipping now (`graffy metrics`, `--json` for machines)** — all folded from
journaled events, nothing estimated: run outcomes; token totals; model/tool
call counts; IU counts; failure-mode frequencies (per run and aggregate);
repair counts, per-op breakdown, and token cost; evidence-level
distribution; escalation efficacy (success rate with vs. without ladder
escalation — `null` when a cohort is empty, never a fabricated rate);
convergence (visit-cap hits, max node visits, mean escalations/run); external
attempt groups and attempts-to-PASS; separately named run-passed and
target-failure-resolution rates for repairs; HRDM sample and rating-journal
counts; approval outcomes.

**Next (in priority order)**:
1. **Convergence curves beyond the shipped aggregate**: full
   attempts-to-PASS distributions, verdict deltas per retry, and
   cost-to-convergence in tokens/seconds.
2. **Repair efficacy by pairing**: target-failure resolution per (failure
   mode × repair op) pair — *the* core empirical claim of the framework,
   without conflating a passing run with causal resolution.
3. **Detection latency**: events between failure introduction and its
   FailureSignal (proxy: node distance draft→verify).
4. **HRDM time series and reliability statistics** over the human calibration
   samples now collected, followed by model samples once C3 lands (§4).
5. **Real USD costs**: user-editable pricing tables arm the `cost_usd`
   budget leg (kept honestly 0.0 until then).
6. **Cross-model comparison**: identical graphs re-run across tier
   bindings (`GRAFFY_MODEL_*`) — the spec SHA pins the procedure, so model
   is the only variable. Cheap, controlled, publishable.

## 4. HRDM operationalization (canon exists; adaptation drafted)

`HrdmSample` is deliberately an **anchored ordinal score** (+
`rubric_version` + `rater_id` + `source`), not a computed statistic —
scoring coordination health is a judgment, and pretending otherwise would
fabricate data. The anchors themselves are NOT graffy's to author: the
framework already carries them (0–3 scales, behavioral anchors for all
four proxies, rater instructions, inter-rater reliability protocol, and a
falsification rule) in mcw-framework `docs/experiments/hrdm_rubrics.md`.
graffy's job is the **rating-substrate mapping** — where in a journal the
canonical units live — drafted as a declared Article V extension in
`docs/mcw/hrdm-in-graffy.md` (awaiting the author's ratification).

Collection paths:
1. `graffy rate <session>` — **SHIPPED v1**: human scores proposed windows
   (H/D/M) and repair episodes (R) against the anchors; unratable units are
   recorded as absent; samples land in rating journals with rater +
   rubric_version (`HUMAN_RATER`; calibration-only while the adaptation is
   draft).
2. A rater graph (C3 kin) — a model scores the same journals against the
   same anchors (source: MODEL).
3. **Inter-rater agreement between 1 and 2 is itself a publishable
   result**: can models reliably score coordination health? Weighted kappa
   over shared journals; disagreements become rubric refinements.

## 5. Benchmark protocol (how researchers verify MCW helps)

The controlled comparison graffy makes cheap: identical task suite, both
arms journaled, `graffy metrics --json` per arm, diff the aggregates.

- **Arm A (floor off)**: a minimal graph with a single `model` node — never a
  raw execution shortcut — with a journal.
- **Arm B (floor on)**: `intake → ground → draft → verify → respond`, C1
  detectors active; with C2, repairs active.
- Primary outcomes: task success rate, unresolved-failure rate,
  cost-per-success (tokens now, USD once pricing lands).
- Secondary: failure-mode profile shifts, escalation efficacy, convergence
  cost, evidence-level mix (does grounding rise?).
- Replication: everything needed travels as files — specs (SHA-pinned in
  every manifest), journals, metrics JSON. A **research bundle** is exactly
  that directory; `graffy bundle` (future) zips it with a manifest. The
  alpha.1 field anecdote (verify caught an ungroundable claim, run failed
  honestly instead of hallucinating) becomes a measured rate, not a story.

## 6. Division of labor

**graffy (any capable agent, guided by AGENT.md)**: C3/C4 code, model-rater
graph, reliability statistics, bundle export, pricing tables, and fuller
curves. C2 and the first human `graffy rate` path are shipped.
**@rainmana (framework author)**: ratify `docs/mcw/hrdm-in-graffy.md` (§4), task suites
for the benchmark arms (10–30 prompts with pass criteria per domain),
choice of first target venue/format for the research bundle.
