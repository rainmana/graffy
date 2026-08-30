# Phase 3 design: the MCW learning loop & research instrument

> Status: ACTIVE. C1 (detectors) and C5 v1 (`graffy metrics`) are shipped;
> this doc is the map for the rest. It also serves as the **MCW coverage
> audit**: what of the framework is implemented, what remains, and what data
> graffy can put in front of other researchers. Companion reading:
> `AGENT.md` (how to work on this repo), `docs/design/phase-2-mcp.md`
> (the facade/taxonomy design), and the framework itself:
> <https://rainmana.github.io/mcw-framework/> (W. Alec Akin, Apache-2.0).

## 1. MCW coverage matrix (honest audit, 2026-08-30)

Legend — **Schema**: typed in `src/protos/mcw.proto`. **Journaled**: emitted
at runtime into run journals. **Acted on**: changes execution behavior.
**Measured**: aggregated by `graffy metrics`.

| MCW construct | Schema | Journaled | Acted on | Measured |
| --- | --- | --- | --- | --- |
| Information Units (typed, salience, provenance) | ✅ | ✅ every run | ✅ ledger feeds nodes | ✅ counts |
| IU five-stage lifecycle (selection→integration) | ✅ `IuStageRecord` | ⚠️ stages recorded only where nodes set them | ⬜ | ⬜ per-stage fidelity |
| Failure mode: Drift | ✅ | ✅ judge-named (C1) | ⬜ (C2 feeds it back) | ✅ frequency |
| Failure mode: Asymmetric State Advancement | ✅ | ✅ judge-named (C1) | ✅ **prevented** — effector approval gates | ✅ frequency |
| Failure mode: False Alignment | ✅ | ✅ judge-named (C1) + interview contradiction probe | ⚠️ interview: conservative annotation wins | ✅ frequency |
| Failure mode: Overcompression | ✅ | ✅ judge-named (C1) | ⚠️ digest nodes carry preserve-distinctions knowledge | ✅ frequency |
| Failure mode: Constraint Opacity | ✅ | ✅ judge-named (C1) | ✅ tool-shape drift warning at run start | ✅ frequency |
| Failure mode: Repair Suppression | ✅ | ✅ judge-named (C1) | ⬜ | ✅ frequency |
| Convergence exhaustion (not a canonical mode — kept `Unspecified`, never force-fit) | ✅ | ✅ visit-cap signal (C1) | ✅ halts the path | ✅ cap-hit runs |
| Repair op: Re-grounding | ✅ node kind | ⚠️ event exists; no built-in emits yet | ⬜ C2 wires detection→repair | ✅ counts+token cost (ready) |
| Repair op: Decompression | ✅ node kind | ⚠️ same | ⬜ | ✅ (ready) |
| Repair op: Re-weighting | ✅ node kind | ⚠️ same | ⬜ | ✅ (ready) |
| Repair op: Disambiguation | ✅ node kind | ⚠️ same | ✅ `PauseForDisambiguation` + interview follow-ups | ✅ (ready) |
| Repair op: Synchronization | ✅ node kind | ⚠️ same | ⬜ | ✅ (ready) |
| H/R/D/M observables | ✅ `HrdmSample` (anchored ordinal scores + rubric version + rater) | ⬜ **needs anchor rubric** (§4) | ⬜ | ✅ sample counts (fold ready) |
| Evidence levels L0–L3 | ✅ | ✅ every artifact | ✅ strict vs trace-only policy | ✅ distribution |
| Evidence artifacts (hash-addressed) | ✅ | ✅ | ✅ ground-before-draft floor | ✅ counts |
| McwStateSnapshot | ✅ | ⬜ | ⬜ | ⬜ |

**Summary for the impatient**: the *observability* layer of MCW is in and
measured (IUs, six failure modes via judge detection, evidence levels,
convergence). The *repair* layer is typed and executable but not yet driven
by detection (that is exactly C2). The *H/R/D/M scoring* layer needs its
anchor rubric authored by the framework's author — the schema is ready and
waiting (§4). Detection today is judge-based (one detector); per-mode
heuristic detectors are future work that the journal format already supports.

## 2. What ships the learning loop (C2–C4)

- **C2 — retry with feedback** (`graffy run --retry n|auto`): when a run
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
convergence (visit-cap hits, max node visits, mean escalations/run); HRDM
sample counts; approval outcomes.

**Next (in priority order)**:
1. **Convergence curves** (C2): attempts-to-PASS distributions; verdict
   deltas per retry; cost-to-convergence in tokens/seconds.
2. **Repair efficacy**: post-repair PASS rate per (failure mode × repair
   op) pair — *the* core empirical claim of the framework, testable.
3. **Detection latency**: events between failure introduction and its
   FailureSignal (proxy: node distance draft→verify).
4. **HRDM time series** per session once sampling lands (§4).
5. **Real USD costs**: user-editable pricing tables arm the `cost_usd`
   budget leg (kept honestly 0.0 until then).
6. **Cross-model comparison**: identical graphs re-run across tier
   bindings (`GRAFFY_MODEL_*`) — the spec SHA pins the procedure, so model
   is the only variable. Cheap, controlled, publishable.

## 4. HRDM operationalization (needs the framework author)

`HrdmSample` is deliberately an **anchored ordinal score** (0–4 ints +
`rubric_version` + `rater_id` + `source`), not a computed statistic —
scoring coordination health is a judgment, and pretending otherwise would
fabricate data. What's missing is the **anchor rubric**: for each of
H(ealth), R(epair cost), D(rift), M(isattribution), written descriptions of
what a 0/2/4 looks like, versioned so scores stay comparable.

Collection paths once the rubric exists:
1. `graffy rate <journal>` — human scores a finished run against the
   anchors (source: HUMAN).
2. A rater graph (C3 kin) — a model scores the same journals against the
   same anchors (source: MODEL).
3. **Inter-rater agreement between 1 and 2 is itself a publishable
   result**: can models reliably score coordination health? Weighted kappa
   over shared journals; disagreements become rubric refinements.

## 5. Benchmark protocol (how researchers verify MCW helps)

The controlled comparison graffy makes cheap: identical task suite, both
arms journaled, `graffy metrics --json` per arm, diff the aggregates.

- **Arm A (floor off)**: single `model` node — a raw prompt with a journal.
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

**graffy (any capable agent, guided by AGENT.md)**: C2/C3/C4 code, rater
graph, `graffy rate`, bundle export, pricing tables, curves.
**@rainmana (framework author)**: HRDM anchor rubric v1 (§4), task suites
for the benchmark arms (10–30 prompts with pass criteria per domain),
choice of first target venue/format for the research bundle.
