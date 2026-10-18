# Release readiness and next development slices

Snapshot: 2026-10-17. Reviewed source: `2088037f3773944cf57d26b86e6ceb123aba212e`.
The original audit below is retained as a dated baseline. The release fixes now
implemented in this working tree are summarized first.

## Alpha.6 implementation update

All seven reproduced blockers have fixes and durable regression coverage:
policy validation, exclusive journal creation, reject-and-resubmit edits,
nonzero failure exits, no failed-run answer release, one count per retry, and
unknown-session rejection.

Additional completed work:

- Exact approval payloads/identity/hash, verbatim frontend decision input, and
  distinct abandonment; no artificial human turns from automatic approvals.
- Model/tool/approval deadlines, cancellation on CLI signal/TUI quit, closed
  approval-channel handling, separate configurable per-call output caps with
  estimated prompt reservation, and whole-turn retry budgets. TUI setup also
  consumes elapsed budget.
- Strict history loading, sequence/run identity checks, duplicate conflict
  detection, and indexed custom journal continuation.
- Honored quality-failure halt policy, exact-draft review identity, request plus
  consumed-source review context, and a verify gate in newly generated MCP facades.
- Output receipts after write/flush, conservative legacy visibility, explicit human
  corrections, open-episode R censoring, complete proposal snapshots, reviewed
  exchange/window/episode corrections and scoped citation checks.
- New repairs no longer claim target resolution. Legacy mode-recurrence metrics
  are explicitly named. The versioned adaptation freezes prospective comparison
  rules and stays draft/calibration-only.
- Release publication now depends on fmt/Clippy/Linux/macOS tests and package
  builds, checks tag/version agreement, runs offline binary smoke journeys,
  normalizes archive ownership, and marks alpha releases as prereleases.

See [ADR-0011](adr/0011-release-and-boundary-receipts.md),
[the v2 adaptation delta](mcw/alpha6-adaptation.md), and [CHANGELOG](../CHANGELOG.md).
Pinned rubric anchors and the historical MCW author review remain unchanged.

Remaining limits are explicit: no pricing tables or true spend cap, no guarantee
that cancellation undoes external work, input token accounting after provider
completion, no semantic claim-to-source proof, no fine-grained IU decomposition,
no completed MCW reliability study. Existing stored facades require regeneration
to gain the new review gate. Desktop/web remains tabled; Ratatui/CLI chat is next.

Final local verification and publication status are recorded at the end of this
document. The original source locations and issue snapshot below describe the
pre-fix commit, not the current tree.

## Original audit (before these fixes)

## Current position

GitHub `main` matched this checkout when checked. The latest published release is
[`v0.1.0-alpha.5`](https://github.com/rainmana/graffy/releases/tag/v0.1.0-alpha.5)
(2026-08-30); the workspace version is `0.1.0-alpha.6`. Seven commits since that tag
add retry feedback, the HRDM adaptation, human ratings, boundary projection,
session continuation, distinct attempt identities, segmentation audit records,
and revised documentation. These changes are committed upstream but untagged.
The [CI run for the reviewed commit](https://github.com/rainmana/graffy/actions/runs/33838066534)
passed.

The graph execution foundation is usable. The largest gap between the intended
product and today's interface is conversational interaction: users currently
launch individual graph runs and inspect journals. There is no chat command,
message composer, conversation list, or prompt-to-graph router.

The owner's confirmed sequence is bug fixes and MCW integration first, then a
chat-like interface where every prompt executes as a graph. Ratatui and CLI are
the interfaces; desktop/web is tabled indefinitely. See [ADR-0010](adr/0010-chat-through-graphs.md).

## What is implemented

| Area | Current implementation | Important boundary |
| --- | --- | --- |
| Execution | TOML parser, topology compiler, guarded cycles, visit caps, model/tool/approval traits, tier escalation, per-run budgets | Serial queue; richer branch/join semantics, hard deadlines, and cancellation need attention before more elaborate generated graphs |
| Journals | Protobuf event stream, live event tap, replay, hashes, run/session/attempt identities | Replay inspects events; it does not resume execution. Exact TOML snapshots are not embedded in journals |
| Providers | Anthropic, OpenAI, OpenRouter, Venice, Ollama through environment-bound tiers | Complete responses, not token streaming; USD costs remain zero |
| MCP | Stdio discovery, usage interview, tool facade graphs, conservative approval gates, real protocol fixture tests | No HTTP transport; stored command arguments lose complex quoting; approval payload needs improvement |
| Graph creation | Four built-ins, registry import/export, skill/prompt graphification, guided and collaborative review | Graphify wraps a fixed floor; model-assisted decomposition and structural editing are future work |
| TUI | Journal browser, live run view, step inspector, graph review, approval modal | No chat; `--session` and retries are explicitly unavailable in live TUI mode |
| Storage | Embedded libSQL graph/MCP registry, session/run indexes, mirrored events | No episodic FTS, embeddings, temporal knowledge graph, learned lessons, or compaction |
| MCW | Typed IUs, support artifacts, failure hypotheses, retry repair records, boundary projection, human HRDM calibration ratings | Instrumentation/test bed, not validated MCW efficacy or a ratified measurement instrument |

The active orchestration is in `graffy-core` and `src/main.rs`. `graffy-mcw`
currently exports types and repair-kind names; the five `repair.*` names are not
registered executable node behaviors. `graffy-agents` and `graffy-evals` remain
scaffolds. Their crate names and architecture prose should not be read as shipped
features.

Useful entry points:

- `src/main.rs`: command dispatch, `run_graph`, retry loop, session loading,
  `cmd_rate`, indexing, provider/tool assembly.
- `crates/graffy-core/src/exec.rs`: node behaviors and `Executor::run`.
- `crates/graffy-core/src/boundary.rs`: external transcript projection and history hydration.
- `crates/graffy-core/src/metrics.rs`: separate execution, attempt-group, session,
  and rating metrics.
- `crates/graffy-tui/src/ui.rs` and `state.rs`: live event consumption, approvals,
  and presentation state.
- `crates/graffy-memory/src/lib.rs`: persistence and registries.

## Fix before the next release

These findings were reproduced against the locally built binary or compiled core.
Priorities below are recommendations for this release, not existing GitHub labels.

| Priority | Finding and reproduction | Relevant source at reviewed commit | Required result |
| --- | --- | --- | --- |
| P1 | An invalid evidence floor silently disables the structural check. With the same offline graph, `L3` fails, but `INVALID` succeeds | `exec.rs:145`, `exec.rs:931`; spec parsing/compiler lack policy validation | Reject unknown policy values before any execution or MCP connection |
| P1 | Reusing `--journal` succeeds and overwrites the old journal | `journal.rs:57` uses `truncate(true)` | Exclusively create new journals; preserve existing bytes on collision |
| P1 | Approve-with-edit releases the original answer. Entering `e AUDIT_REPLACEMENT` still returns `AUDIT_ORIGINAL` | `exec.rs:1535`, `exec.rs:1588`, `RespondNode`; `main.rs:1930` | Define edits as applied corrections with revalidation, or remove/rename the misleading action until supported |
| P1 | Failed and cancelled runs return exit code zero. A strict-floor failure and a rejected approval both reproduce it | `main.rs:679-704`; completed TUI outcomes also return success | Report non-success outcomes through meaningful process exit codes after preserving/indexing the journal |
| P1 | A graph with a model node after `respond` can exceed its budget and still print a **verified response**. The journal omits the corresponding AI boundary turn because the run failed | `main.rs:767`; `exec.rs:1680` | Align release, terminal output, and boundary records; never release a failed run as verified |
| P2 | One automatic retry with explicit boundary events is reported as two retry runs by the boundary projection | `boundary.rs:331` and `boundary.rs:352` | Increment once; test the explicit-event path, not only legacy reconstruction |
| P2 | `--session audit-session-that-does-not-exist` succeeds with empty history | `main.rs:603`, `load_session_runs` | Reject an unknown continuation target; distinguish new-session creation from resume |

The retry-count defect affects the boundary projection's internal telemetry shown
by unblinded `graffy rate`; it does not establish that the separate
`attempt_groups_from_rows` metrics fold double-counts attempts.

The following additional risks are confirmed by source inspection; they were not
reproduced with interactive terminal or live-provider tests during this audit:

- **Approval context:** the handler receives a question and node ID, not the
  proposed answer or prepared tool arguments. Plain CLI approval cannot review
  the actual action. Show a concrete payload, preserve its identity, and apply
  edits before execution. This matters especially for MCP effectors.
- **TUI quit before a future approval:** `run_live` leaves the UI loop and awaits
  the task while retaining `approval_rx`. If the task later requests approval,
  its reply can wait with no active UI consumer. Close/reject the channel or
  implement explicit cancellation; distinguish “leave view” from “stop run.”
- **Budgets are checked between completed nodes:** an awaited provider/tool call
  can outlive `max_seconds`. Requests receive `max_tokens: None`; retries reset
  per-run budgets. Add call deadlines, remaining-token limits where supported,
  and an explicit whole-turn retry budget. Token usage returned after a call is
  accounting, not a hard pre-call cap.
- **History loading silently drops errors:** unreadable/corrupt journals are
  skipped; continuation only scans the default runs directory, so a custom
  `--journal` elsewhere can be indexed yet absent from history. Surface incomplete
  history and use the session index to locate relevant journals.
- **Policy coverage:** `on_quality_fail` is stored but ignored. An unsupported
  configuration should be rejected rather than appear to control execution.
- **Verification context:** the judge sees the draft, not the original request
  and full supporting context. The structural floor checks the strongest
  inherited artifact, which can be the user's prompt. This is not claim-level
  grounding or proof of completeness. MCP facade graphs also omit a verify node,
  although `respond` is labeled `verified_response` generically.

Keep the WSL overlap issue [#15](https://github.com/rainmana/graffy/issues/15)
parked under its existing reproduction requirement. No visual TUI inspection was
performed in this audit.

## MCW: implemented repairs and remaining work

The recent changes materially improve the measurement substrate:

- Operational `SupportLevel` and study-level `McwEvidenceLayer` are separate.
- Normal execution no longer fabricates perfect five-stage fidelity; assigned
  salience is labeled as a policy prior.
- Judge labels carry source artifacts and absent confidence instead of a fixed
  pseudo-measurement.
- Boundary turns, automatic attempts, and rating observations have distinct
  identities. Continuation hydrates prior visible conversation into the next run.
- Rating displays pinned anchors and transcripts, records citations and required
  unratable reasons, supports blinding, and persists segmentation decisions.

Remaining work should be separated into conformance and later research features:

1. **Make boundary records faithful to interaction.** Explicit rejection and EOF
   currently share one outcome, so deliberate human rejections are omitted from
   the boundary transcript. Normal follow-up prompts always become Goal IUs;
   there is no CLI/chat input role for an explicit correction to initiate a repair
   episode. A response event is written before presentation acknowledges delivery.
   Define produced, released, delivered, and unknown states without claiming the
   user's attention or understanding was observed.
2. **Finish support and repair semantics.** Evidence inheritance is broad
   consumed-IU lineage, not a tested claim-to-source map. Whole-prompt IUs remain
   too coarse for fine-grained late-discovery claims. `target_failure_resolved`
   uses same-mode non-recurrence plus run success as a proxy, not demonstrated
   continuity of the same misalignment. Keep that distinction visible in metrics.
3. **Complete rating review before ratification.** Window segmentation allows
   accept/exclude; episode adjustment can mark an episode open, but arbitrary
   boundary correction is not implemented. Freeze the adaptation's segmentation
   disagreement rules and evaluate the declared reliability gates. Current output
   is correctly labeled draft/calibration-only. Code passing tests does not ratify
   the instrument or validate the framework.
4. **Then expand capability.** Independent/heuristic detectors, executable repair
   graphs, critique meta-evaluation, model raters, learned lessons, recall,
   compaction, pricing, and reproducible research bundles remain separate work.
   They need not all block a useful first chat release.

The pinned adaptation document reviews an older commit and remains historical
evidence. Do not silently change pinned rubric text, reinterpret old samples, or
remove draft status as part of a routine release.

## Proposed delivery sequence

1. **Finish alpha.6 as a reliability and MCW-recording update.** Add regression
   coverage for the reproduced bugs, align release/boundary semantics, make
   approvals concrete, and define cancellation and budget behavior. Update the
   changelog to describe the final behavior and remaining calibration status.
2. **Add chat using the existing default conversation graph.** Share execution
   orchestration between CLI and Ratatui; one session spans many user turns, one
   attempt group belongs to each turn, and retries remain internal. Ship a composer,
   visible transcript, resume/new-session actions, approvals, cancellation, and
   a collapsed progress view. See ADR-0010 for acceptance criteria.
3. **Add prompt-to-graph routing.** Continue
   [#17](https://github.com/rainmana/graffy/issues/17): explicit graph selection
   wins; a journaled router may choose eligible registry graphs, with the
   conversation floor as fallback. Honor explicit-only adoption policy. Start
   without model-generated graph structures.
4. **Add optional graph detail and richer generation.** Reuse journal events for
   the reasoning/inspection layer. Model-assisted graphify [#7](https://github.com/rainmana/graffy/issues/7)
   and structural editing [#8](https://github.com/rainmana/graffy/issues/8) are
   later increments, with compiler checks and review of proposed effects.

Long conversations need an explicit context policy: initially warn and stop before
context overflow or disclose a bounded selection. Do not silently drop earlier
constraints. IU-aware compaction can follow as a graph with its own evidence.

## Backlog and release hygiene

All 17 GitHub issues were open at review time. Some describe older requirements:

- [#1](https://github.com/rainmana/graffy/issues/1): retry v1 exists; split remaining
  total-budget, no-improvement stopping, and human-repair work from completed work.
- [#4](https://github.com/rainmana/graffy/issues/4): still asks for 0-4 anchors and
  `rate <journal>`; code now uses pinned 0-3 anchors and session ratings. Reconcile
  the issue with the draft adaptation and residual review work.
- [#17](https://github.com/rainmana/graffy/issues/17): already captures the owner's
  prompt-first idea. Extend its implementation plan rather than create a competing
  feature definition.
- #2/#3: meta-evaluation and lessons; #5/#6: pricing and bundles; #7/#8: generated
  structure and editing; #9/#11: TUI polish and accessibility; #10: HTTP MCP.
  Signing, LangGraph export, and self-administration remain tabled (#12-#14).
- ROADMAP, ARCHITECTURE, the Phase 3 matrix, and early alpha.6 changelog entries
  contain stale claims about evidence layers, retry episodes, scoring provenance,
  and implemented crates. Rewrite current claims while preserving dated review
  records. The latest README is more candid but also omits newer CLI features.
- Release workflow still lacks a display name (#16), does not gate publication
  on the test workflow itself, and does not normalize archive ownership as
  recommended in the conformance review. No workflow was changed here.

Before tagging, run the repository's fmt/clippy/test gates on the final commit,
confirm Linux/macOS CI, smoke the packaged binaries, verify the tag points to that
commit and matches the reported version, and review the release notes. Signing
and new platform targets are not prerequisites for this scope.

## Verification performed

- `cargo fmt --all --check`: passed.
- `cargo +stable-x86_64-pc-windows-gnullvm clippy --workspace --all-targets --locked -- -D warnings`: passed.
- `cargo +stable-x86_64-pc-windows-gnullvm test --workspace --locked --no-fail-fast`:
  **69 passed, 0 failed, 1 ignored**. The ignored test requires a real network MCP
  server; the hermetic MCP fixture passed.
- Local Windows setup: the repository's default stable MSVC toolchain could not
  find `link.exe`. The already-installed GNU LLVM host toolchain built successfully.
  A local test-only `python3.exe` launcher under ignored `target/release-audit/bin`
  forwarded to installed Python because the Windows Apps alias was unusable. No
  system tools were installed and no production source was changed.
- Black-box offline CLI checks: init, default conversation, rejection, edit,
  invalid/strict support floors, journal collision, unknown-session continuation,
  and a failed run retaining response text. A compiled core probe confirmed the
  explicit-boundary retry count defect.
- Audit outputs and reproduction helpers are local ignored files under
  `target/release-audit/`, including `cli-results.json` and `workspace-tests.txt`.
  These are not shipped artifacts or durable regression tests.
- No live model-quality, real paid-provider, interactive TUI rendering, or
  cancellation test was performed. Existing green CI covers Linux/macOS; the
  local Windows run does not establish supported Windows distribution.

The original audit changed documentation only. The implementation update above
supersedes that original status. No issues have been closed and no release has
been published by this local preparation.

## Final alpha.6 verification (2026-10-17)

- Formatting: `cargo +stable-x86_64-pc-windows-gnullvm fmt --all --check` passed.
- Clippy: `cargo +stable-x86_64-pc-windows-gnullvm clippy --workspace --all-targets --locked -- -D warnings` passed.
- Full workspace tests: **100 passed, 0 failed, 1 ignored**. The ignored test is
  the opt-in network/npx MCP server. The real-stdio hermetic MCP fixture passed,
  as did a facade revision test proving that review retries do not repeat a tool effect.
- Optimized build: `cargo +stable-x86_64-pc-windows-gnullvm build --release --bin graffy --locked` passed;
  binary reports `graffy 0.1.0-alpha.6`.
- Ten optimized-binary offline checks passed: version, init, graph list, run,
  replay, journal-collision rejection, indexed custom-journal continuation with
  correction, blinded open-episode rating, approval rejection, and JSON metrics.
- Regression coverage includes the original seven blockers, remaining budgets
  across retries, stalled model/tool/approval deadlines, cancellation, exact
  review-context scope, failed output flushes, stale draft review identity,
  visit-cap failure after a response, history integrity, actual approval input,
  corrected segmentation, citation validation, and atomic rating aborts.
- Ratatui tests cover small-terminal payload rendering/scrolling, rejecting the
  removed edit key, and closing current/future approval requests. These are backend
  and lifecycle tests, not a human visual inspection of Windows Terminal/WSL.
- Workflow YAML structure and the embedded archive-packaging script were checked
  locally. The fixture archive contains the expected four files with normalized
  ownership. Hosted GitHub Actions has not run for these changes yet.
- Pinned anchor text and the historical MCW conformance review are unchanged.

The local Windows MSVC linker is unavailable, so all Rust checks used the installed
GNU LLVM toolchain. The MCP fixture used the installed Python through an ignored
test launcher. Final logs and local smoke helpers are under `target/release-audit/`;
the regression tests themselves are committed source. No live model-quality or
paid-provider calls were made. Windows remains a development check, not a new
supported binary-distribution target.

Prepared on `codex/alpha6-readiness` and submitted as
[PR #19](https://github.com/rainmana/graffy/pull/19). Before publishing alpha.6,
review/merge the branch, require Linux/macOS CI on the accepted commit, then tag
that exact commit `v0.1.0-alpha.6`. The tag workflow independently reruns gates and
checks the three target binaries before publication. Chat remains the next
Ratatui/CLI task.

### PR review follow-up: per-call token limits

Codex correctly identified that passing the default 200,000-token run budget as
a provider's generation cap can reject otherwise valid live calls. A capped
Rig test model reproduced that failure before the fix. The request now names
the aggregate allowance separately; the live adapter applies a configurable
per-tier output cap and reserves estimated input usage (ADR-0011).

Five new tests cover the original rejection, cap configuration, system/Unicode
prompt reservation, avoiding provider dispatch when the input reserve exhausts
the allowance, and the full default graph's success/budget-exhausted journals.
After this follow-up, formatting and workspace Clippy pass, and the locked
workspace suite has **105 passed, 0 failed, 1 ignored** on Windows GNU LLVM.
These use local fakes; no paid-provider call or model-limit discovery is claimed.
