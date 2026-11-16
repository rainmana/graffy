# Decision-model routing: setup

`graffy chat --route=decider` asks a small **decision model** which opted-in
graph each message should run on. This page is the shortest path to a working
setup. Design and rules: [ADR-0012](adr/0012-prompt-to-graph-routing.md).

A decision model does not write text. It picks one of the choices it is given
and reports how confident it is. graffy offers it the graphs that declared
`route_on`, plus plain conversation, and follows the pick only if the reported
confidence reaches the threshold **you** set.

## 1. Choose where the model runs

| | Hosted: Jev (TypeSafe AI) | Local: Strands Decider |
| --- | --- | --- |
| Your messages go to | `api.typesafe.ai` | Your own machine only |
| Needs | An API key | Python 3.10+, several GB of disk and memory |
| Status in graffy | Exercised end to end | **Not yet run against graffy** — see below |

## 2a. Hosted Jev

```console
$ export GRAFFY_DECIDER_URL=https://api.typesafe.ai
$ export GRAFFY_DECIDER_MODEL=<a name from GET /v1/models>
$ export GRAFFY_DECIDER_API_KEY=<your key>
$ export GRAFFY_ROUTER_MIN_CONFIDENCE=<a number from 0 to 1>
```

graffy never hardcodes a model name. List the names your account can use:

```console
$ curl -s https://api.typesafe.ai/v1/models -H "Authorization: Bearer $GRAFFY_DECIDER_API_KEY"
```

## 2b. Local Strands Decider

[Strands Decider](https://github.com/strands-labs/strands-decider) is an
Apache-2.0 family of open decision models with a small HTTP server. The
commands below are **from its own documentation**. graffy's adapter was written
against its published request/response schema, but nobody has yet run the two
together, so treat this section as the expected path and report what happens.
Its documentation also says compatibility with the Jev API "is not verified".

Start the server. With [uv](https://docs.astral.sh/uv/) nothing needs
installing first:

```console
$ uvx strands-decider serve StrandsAgents/strands-decider-2B-qwen3.5-v1-2610 --port 8000
```

or with pip:

```console
$ pip install strands-decider
$ strands-decider serve StrandsAgents/strands-decider-2B-qwen3.5-v1-2610 --port 8000
```

- The first start downloads the model weights from Hugging Face.
- Add `--device cpu` to force CPU; on Apple silicon it uses the GPU (`mps`) by
  default. Its README lists other sizes and when to pick them.
- It listens on `127.0.0.1` only and **has no authentication**. Upstream says
  to use it for local experiments only. Do not expose it to a network.

Check that it is up (in another terminal):

```console
$ curl -s http://127.0.0.1:8000/health
```

Then point graffy at it. No model name or key is needed:

```console
$ export GRAFFY_DECIDER_URL=http://127.0.0.1:8000
$ export GRAFFY_ROUTER_MIN_CONFIDENCE=<a number from 0 to 1>
```

Plain `http://` is accepted only for `localhost`, `127.0.0.1` and `[::1]`.

## 3. Choose the threshold

`GRAFFY_ROUTER_MIN_CONFIDENCE` is required and has no default. A pick the model
reports below it is not followed; plain conversation runs instead.

graffy has no evidence of its own about either model's accuracy on your
prompts, which is why it does not choose for you. One published reference
point: the Strands Decider README states that, on short classification tasks
its models had not seen, answers at a confidence of 0.9 or more were right
about 95% of the time. That claim is theirs, about their tasks.

A higher number means fewer messages are routed and fewer are routed wrongly.

## 4. Check, then chat

```console
$ graffy doctor
$ graffy chat --route=decider
$ graffy chat --tui --route=decider
```

`graffy doctor` reports whether the decider settings are usable and where
messages would be sent, without contacting the endpoint.

Chat states which endpoint receives routed messages **before** any message is
sent. Start a message with `/chat` to keep it out of routing.

## What gets recorded

Each routing step is its own journaled run. It records the model's reported
choice, confidence and per-choice probabilities, the model that answered, the
endpoint, your threshold, and the reason shown to you. The confidence is the
model's own report, not a graffy measurement. `graffy metrics` counts routing
runs and their tokens separately from execution runs.

## When it does not work

graffy fails the turn loudly and never falls back to trigger phrases.

| Message contains | Meaning |
| --- | --- |
| `GRAFFY_DECIDER_URL is not set` | No endpoint configured |
| `graffy ships no default` | `GRAFFY_ROUTER_MIN_CONFIDENCE` is missing |
| `uses plain http:// for a non-local host` | Use `https://`, or a local address |
| `must not contain a username or password` | Put the key in `GRAFFY_DECIDER_API_KEY` |
| `routing failed: … request failed` | The endpoint could not be reached |
| `routing failed: … HTTP 401` / `403` | The key was rejected |
| `routing failed: … HTTP 422` | The service rejected the request, often a missing or unknown model name |
| `which is not one of the offered graphs` | The model answered outside the choices it was given |

## Making a graph routable

```toml
[graph]
id = "example.decide"
name = "Decide"
version = "0.1.0"
route_on = ["pros and cons", "help me decide"]
route_when = "The person wants help choosing between options."
```

`route_on` opts the graph in (and is what `--route` without a model matches).
`route_when` is the sentence the decision model is given; the graph's
description is used when it is absent. Graphs without `route_on` are never
offered.
