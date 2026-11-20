# Live-provider smoke test

The required gates (`cargo test --workspace`, and CI) never talk to a model:
they run on the deterministic `OfflineEcho` invoker and in-process fakes. That
keeps them fast and free, but it means the path a real user takes — tier
bindings, a provider key, a real HTTP client, real token usage — is not covered
by them.

`tests/live_provider.rs` covers that path on request. It runs one turn of the
default conversation graph (`intake → ground → draft → verify → respond`)
through the real `graffy` binary without `--offline`, then reads the journal the
run wrote. Nothing is shortcut: the model is reached only through a scheduled
graph node, exactly as in normal use.

It is **opt-in**. The two live tests are `#[ignore]`d, so the required gates do
not run them and CI needs no secrets — the same arrangement as
`cargo test -p graffy-mcp -- --ignored` for the live MCP server.

## The two live tests

| Test | Covers | Needs |
|---|---|---|
| `live_hosted_provider_releases_a_journaled_answer` | a keyed HTTPS provider: `anthropic`, `openai`, `openrouter` or `venice` | tier bindings + that provider's API key |
| `live_ollama_releases_a_journaled_answer` | an Ollama server | tier bindings + a reachable Ollama with the model pulled |

Each test refuses bindings from the other family. The test name is the claim
about what was exercised, so a "hosted" run that actually reached Ollama must
fail rather than pass.

## Environment

The tests read the same variables a real run reads. Nothing is remapped, and no
model name is written in the repository — you choose the model.

| Variable | Purpose |
|---|---|
| `GRAFFY_MODEL_BALANCED=provider:model` | the tier the `draft` and `verify` nodes ask for |
| `GRAFFY_MODEL_FRONTIER=provider:model` | the tier a REVISE verdict escalates to |
| `ANTHROPIC_API_KEY` / `OPENAI_API_KEY` / `OPENROUTER_API_KEY` / `VENICE_API_KEY` | the key for the bound hosted provider |
| `OLLAMA_API_BASE_URL` | optional; where Ollama listens when it is not the default local address |
| `GRAFFY_MAX_OUTPUT_TOKENS_<TIER>` | optional; per-call output cap for a tier, as in normal runs |

Why both tiers: when the verify gate says REVISE, graffy escalates up the
routing ladder instead of bouncing back to the same model. With only
`BALANCED` bound, a REVISE would end the run with "no model bound for tier
'frontier'", which says nothing about the provider. The test checks both
bindings before it starts. Binding both tiers to the same model is fine.

`GRAFFY_MODEL_FAST` is not needed: the default conversation never asks for it.

## Running it

A hosted provider (macOS, Linux, WSL):

```console
$ export ANTHROPIC_API_KEY=...            # or the key for the provider you bind
$ export GRAFFY_MODEL_BALANCED=anthropic:<model>
$ export GRAFFY_MODEL_FRONTIER=anthropic:<model>
$ cargo test --test live_provider live_hosted -- --ignored --nocapture
```

Ollama:

```console
$ export GRAFFY_MODEL_BALANCED=ollama:<model>
$ export GRAFFY_MODEL_FRONTIER=ollama:<model>
$ cargo test --test live_provider live_ollama -- --ignored --nocapture
```

Windows PowerShell (one statement per line):

```powershell
$env:GRAFFY_MODEL_BALANCED = "ollama:<model>"
$env:GRAFFY_MODEL_FRONTIER = "ollama:<model>"
cargo test --test live_provider live_ollama -- --ignored --nocapture
```

Run the tests by name, as above. Both cannot pass under one environment,
because a tier is bound to one provider at a time; `-- --ignored` without a name
runs both and the one whose family is not bound fails with instructions.

`--nocapture` prints one summary line: how many provider calls were made, to
which `provider:model`, and the token counts the provider reported.

A hosted run spends real tokens: at least two calls (draft and verify), more if
the gate asks for a revision, bounded by the graph's own budget
(`max_tokens`, `max_seconds`).

## What a pass means

- the run wrote exactly one journal, opened by a manifest whose spec hash is the
  built-in conversation graph;
- every journaled model call was answered by a bound `provider:model` of the
  family under test, and reported non-zero input and output tokens;
- the run totals equal the sum of those calls;
- the run ended `Succeeded`, exited 0, released exactly one answer, printed that
  answer, and recorded one output receipt for it.

## What a failure means

The message names the layer:

- **not configured** — a tier is unbound or bound to the other family. The test
  never falls back to the offline echo.
- **no journal** — the run never started; the printed output says why (for
  example `ANTHROPIC_API_KEY is not set`).
- **no provider call completed** — authentication, network or model-name
  problems; the provider's error is in the printed stderr.
- **zero tokens reported** — the provider answered but reported no usage.
  graffy's token budgets are enforced on reported usage, so this is worth
  writing down in `docs/KNOWN-ISSUES.md`, not relaxing in the test.
- **the graph released no answer** — the provider worked, but the verify gate
  kept asking for revisions (or a budget ran out). That is model judgment, not
  transport. Small local models may not follow the reviewer's
  "begin with PASS or REVISE" instruction.

When a test fails, its temporary graffy home is kept and its path is printed, so
you can read the judge's reasons:

```console
$ cargo run -- replay <kept-home>/runs/<run>.journal --events
```

## What it does not cover

One turn of one graph. It does not test model quality, multi-turn sessions,
tool calls, cancellation, timeouts, rate limits, streaming, or pricing
(`cost_usd` is still an honest zero — there are no price tables).

## The hermetic half

The same file holds tests that run in the required offline gate, with no model
and nothing leaving the machine:

- `unreachable_provider_fails_loudly_and_releases_nothing` binds a tier to a
  loopback port that hangs up. The real provider client dials it, the run exits
  non-zero naming a provider error, and the journal records a failed run with no
  model call, no tokens and no released answer.
- `stub_provider_run_passes_the_live_acceptance_check` points the Ollama
  binding at a loopback stub that returns a canned reply. It runs the same
  acceptance check the live tests use, so that check cannot silently rot, and
  it shows that the per-tier output cap reaches the wire and that reported usage
  reaches the journal. The stub is **not** evidence that graffy works with
  Ollama; only the live test is.
- three small tests pin the preflight rules described above.
