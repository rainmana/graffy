# ADR-0011: Release, cancellation, and boundary receipts

Status: Accepted for alpha.6 (2026-10-17). Refines ADR-0003 and ADR-0008.

A successful graph run produces an eligible answer, but cannot establish that
its frontend delivered it. The executor journals a `released` boundary candidate.
The CLI writes and flushes the answer, then appends a `delivered` receipt to that
same journal. The projection merges these records into one turn. A receipt means
output was accepted by the stream, not that a person read or understood it.
If output or receipt writing fails, visibility remains unknown. One frontend owns
delivery; concurrent delivery of the same journal is unsupported.

Only a passing review of the exact draft allows the legacy role name
`verified_response`; the interface calls this a **reviewed response**. Other
successful graphs may release an explicitly unreviewed response. A failed,
cancelled, or budget-exhausted run releases none and the CLI exits nonzero.
Journals are exclusively created and sequence/run identity is checked on read.

Approval displays the exact prepared arguments or draft, IU identity, and hash.
Approval applies to that payload. Edits require rejection followed by a correction
turn and new graph execution. EOF, UI closure, cancellation, and timeouts are
abandonment, not human decisions. Actual CLI input and TUI decision keys are
preserved. Programmatic automatic approvals do not create human boundary turns.

The graph time budget interrupts waiting model calls, tools, and approvals.
CLI retries share remaining tokens, recorded USD cost, and elapsed time; TUI
setup also consumes the run deadline. Quitting the live TUI cancels unfinished
execution and closes its approval channel. Cancellation stops local waiting;
it cannot undo an external tool effect or guarantee a provider stopped work.
Model output ceilings are requested from remaining tokens; input token usage
is known after the call and can exceed the remaining total. USD enforcement
depends on supplied prices; current live invokers supply no pricing.

Reviewers receive the original request and only source IUs the draft consumed.
The graph's reviewer tier determines its provider, which can differ from the
drafter. Consumed-source lineage is not a claim-to-source proof; a prompt receipt
does not establish the truth of facts in a response.

Legacy output records without delivery receipts remain inspectable with unknown
visibility. They do not silently enter confirmed exchanges or continuation
context. Existing rating journals are not rewritten. The calibration substrate
changes are versioned in [the v2 draft delta](../mcw/alpha6-adaptation.md).
