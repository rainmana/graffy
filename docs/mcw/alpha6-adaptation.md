# Alpha.6 journal adaptation delta

Version: `graffy-journal-map@v2-draft`; segmentation `boundary-projection@v3`;
blinding `boundary-v2` (with `+redacted-cli` when requested).
Status: **draft, calibration only**. This implementation does not ratify the
instrument, validate MCW efficacy, or establish study-level L3 evidence.

The [author review and v1 adaptation](hrdm-in-graffy.md) remain historical evidence.
The [pinned anchor text](hrdm-anchors-pinned.md) is unchanged. The source pin is
`mcw-hrdm@8365d220+sha256.87ed2be6`. This delta declares the following changes.

## Boundary observations

Human prompts and explicit `run --session <id> --correction` inputs are distinct.
Internal retries remain internal, with one retry count per run. Approval input
is a human turn only when a human actually responds. An automatic approval,
EOF, closed terminal, or deadline is not a human decision.

A response is counted only with a frontend `delivered` receipt. `released` means
eligible output, not observed delivery. Legacy `visible` response records and
response IUs alone lack this receipt and remain unknown. Unknown contributions
break exchange pairing and are excluded from continuation. Raw journals remain
available for inspection; no past records or samples are relabeled in place.
Delivery proves stream acceptance, not reading, attention, integration, or agreement.
Approval questions become confirmed when the human responds; abandoned questions
remain unknown even if they may have been rendered.

SupportLevel is operational source support, separate from McwEvidenceLayer.
The deterministic floor requires receipts and their recorded support; it does not
prove entailment. Reviewers see the request and consumed source IUs. Fine-grained
claim grounding and validated IU decomposition remain future work. Whole-prompt
IUs are not evidence of measured five-stage fidelity.

New repair records leave `target_failure_resolved` absent: overall success and
same-mode non-recurrence do not demonstrate repair of the same distinction.
Historical proxy values appear in metrics only under `legacy_recurrence_assessed`,
`legacy_mode_not_repeated`, and `legacy_mode_nonrecurrence_rate`. These JSON keys
replace the misleading target-resolution names. Run-passed counts are descriptive,
not causal repair efficacy.

## Reviewing segmentation

`graffy rate SESSION` prints transcript indices and a snapshot SHA-256. The frozen
proposal contains the entire projected transcript, order, visibility, timestamps,
source references, attempt groups, exchanges, windows, and episode boundaries.
The hash detects differences within that snapshot; it is not a signature or
proof of journal authenticity. Internal model/routing/failure labels are excluded.

Use `--segmentation correction.json` to supply reviewed exchange/window/episode
boundaries. The hash must match the current proposal, and a nonempty note is
required. Example for five exchanges, with a previously missed repair initiation:

```json
{
  "proposal_sha256": "COPY_THE_PRINTED_SNAPSHOT_HASH",
  "note": "Turn 2 corrects a missed constraint; both parties proceed at exchange 3.",
  "exchanges": [[0,1], [2,3], [4,5], [6,7], [8,9]],
  "windows": [[0,4]],
  "episodes": [{"initiating_turn": 2, "closed_at_exchange": 3}]
}
```

All indices in the JSON are zero-based. Exchange pairs must be adjacent, ordered,
non-overlapping visible contributions from opposite parties. Windows contain
exactly five consecutive exchanges; partial and overlapping windows are rejected.
An episode starts at a visible human turn, including a prompt the rater identifies
as a correction. Closure requires a later completed exchange; `null` leaves it
open. Multiple distinct episodes require distinct initiation turns. Semantic
continuity and closure are human judgments, not inferred by these validators.
This surface cannot manufacture missing turns, change transcript text, or assert
delivery for unknown records. Such sessions require exclusion or new evidence.

The full correction is persisted separately from the frozen proposal, including
when every scoring unit is excluded. EOF or `q` aborts all recording. Per-window
exclusion and marking an episode open remain available interactively. Only listed
actions are accepted; mistyped actions cannot silently accept a proposal.

H citations must belong to the window. M citations must name visible human turns
in it. D requires ordered introduction/discovery citations, with discovery in the
rated window at least three completed exchanges later; introduction may precede
the window. These are structural checks, not automatic judgments of misalignment
or capability blame. An open episode records R as absent/unratable because its
total repair cost is not observed. R_ev counts confirmed external episode starts,
including open episodes, per ten completed exchanges.

## Frozen prospective calibration comparison rules

These rules apply to a new v2 calibration corpus and must not be retrofitted to
old samples. Before scoring, freeze corpus/session IDs, software versions,
snapshot hashes, eligibility/exclusion decisions, and independent annotations.
Do not expose the mechanical proposal to the independent annotator before that
annotation is saved. The interactive rating UI is a review surface, not proof of
independent annotation; the study operator must provide that separation.

For each session and each class, compare sets of exact units: turns are
`(actor, source refs, content, visibility)`, exchanges are ordered source-ref pairs,
windows are ordered five-exchange lists, and episodes are `(initiation refs,
closure exchange refs or open)`. Match exact units. A boundary, role, visibility,
closure, added-unit or omitted-unit difference is a disagreement. The denominator
is the union of units proposed by either annotation in that class; the numerator
is that union minus the exact intersection. Report class counts and rates
separately, with zero denominators marked unavailable, never zero disagreement.

Exclude a session before comparison only for documented missing/truncated source
records or uncertain boundary visibility; retain its exclusion reason and count.
Report both excluded-session fraction and per-proxy unratability. Do not remove a
unit merely because it disagrees. Preserve both independent annotations and the
original proposal; adjudication produces a third record and cannot improve the
reported pre-adjudication agreement rate. Publish session-level counts as well as
pooled class rates so one long session cannot hide disagreement elsewhere.

After one calibration round, disagreement above 20% in any load-bearing class
fails this adaptation. The parent protocol remains unchanged: at least 12
sessions, two independent raters, 48 H/D/M windows, 24 R episodes, and the parent's
score-agreement and unratability gates. No such study was performed for alpha.6.
Automated agreement analysis, model raters, repair graphs, lessons, fine-grained
IU segmentation, and research bundles remain separate research features.
