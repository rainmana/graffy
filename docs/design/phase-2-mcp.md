# Phase 2 design: MCP servers as graph citizens

> Status: DRAFT for review — captures the design conversation of 2026-08-29.
> The core insight (credit: @rainmana): MCP servers often ship *skills* —
> usage knowledge — and that knowledge must **front** the endpoint, not sit
> beside it. And when no skill exists, graffy should ask the human how they
> actually use the server, because placement-in-a-graph is information the
> protocol cannot carry.

## 1. What an MCP server actually gives us

MCP has three primitives, and they map onto graffy asymmetrically:

| MCP primitive | What it is | graffy mapping |
| --- | --- | --- |
| **Tools** | model-invocable calls with JSON schemas + annotations | the raw material for nodes |
| **Prompts** | server-authored prompt templates | **the server's own skills** — the author telling you how the thing is meant to be used |
| **Resources** | data the server exposes | evidence sources for `ground`-role nodes |

The ecosystem adds a fourth layer the protocol doesn't formalize: SKILL.md-style
agent skills that bundle instructions *about* a server. Both prompts-primitive
skills and sidecar skills are the same thing to graffy: **usage knowledge**.

## 2. Skills front the endpoint: the facade subgraph

A bare tool call is a raw prompt with extra steps — exactly what graffy exists
to prevent. So importing an MCP server does NOT produce naked `tool` nodes.
It produces a **facade subgraph** per tool (or tool family):

```
prepare  →  invoke  →  digest
```

- **prepare** (`model`, tier `fast`): renders the call from graph state using
  the *skill content* as its system knowledge — the server's MCP prompts,
  sidecar SKILL.md, or the user's usage profile (§4). Owns argument
  construction against the tool's JSON schema. A malformed call dies here,
  cheaply, not at the endpoint.
- **invoke** (`tool.invoke`): the transport call itself. No prompting, no
  interpretation. Produces an `EvidenceArtifact` (kind `MCP_RESULT`, level L2
  when the server is authoritative for the domain, L1 otherwise — set by the
  server's registry entry, overridable per graph).
- **digest** (`model`, tier `fast`, optional): converts raw results into IUs
  with the evidence attached — so downstream nodes consume Information Units,
  never raw payload soup (Overcompression guard: digest must preserve the
  distinctions the schema marks salient).

The facade is itself a durable TOML graph (`graffy.mcp.<server>.<tool>`),
exportable and forkable like everything else. Advanced users can bypass to
`tool.invoke` explicitly; the default path is always the fronted one.

## 3. Node taxonomy v2: kind × role

The Phase 1 taxonomy (`intake`, `research`, `model`, `verify`, `respond`,
`approval`, `repair.*`) describes what a node **is**. The screenshot-era
question — "where does this MCP server land in a graph?" — is about what a
node is **for**. That is a second, orthogonal dimension:

- **kind** — the implementation contract (unchanged, plus `tool.invoke` and
  `tool.facade`).
- **role** — the position in the epistemic pipeline:
  - `evidence` — gathers facts; feeds `ground` slots; read-only by nature
  - `effector` — changes the world (sends, writes, deploys); **approval-gated
    by default**, always journaled with its arguments hash
  - `transform` — reshapes data already inside the run
  - `memory` — recall/persist against the store
- **capability tags** — free-form hints for graphification ("web-search",
  "github", "filesystem") used when adopting skills into existing graphs.

Roles are seeded automatically from MCP tool annotations where present —
`readOnlyHint → evidence`, `destructiveHint → effector` (+ mandatory approval
gate unless the user explicitly waives per server) — and confirmed or supplied
by the usage interview when absent. Unknown role defaults to `effector`, the
conservative choice: nothing destructive ever slips in as "just research."

## 4. The usage interview (optional, three questions)

`graffy mcp add <server>` runs discovery (tools, prompts, resources,
annotations). If usage knowledge is missing or ambiguous, graffy *optionally*
asks — plain language, novice-friendly, skippable:

1. "What do you usually use this for?" → role + capability tags
2. "Does it change anything outside your machine, or just look things up?"
   → effector vs evidence + approval-gate default
3. "When should a graph reach for it — always, or only when you say so?"
   → adoption policy (auto-adoptable by graphification vs explicit-only)

Answers persist as a **usage profile** in the server registry (libSQL) and
become the prepare node's knowledge when no server-shipped skill exists.
The interview is itself a graph, obviously.

## 5. Transport lives in the registry, not the spec

Whether a server speaks **stdio or streamable HTTP** is deployment detail.
Graph specs reference servers by logical name (`server = "github"`); the
server registry maps names to transport + endpoint + credentials per
installation. This keeps shared graphs portable: your friend imports your
graph and binds *their* server instance, exactly like model tiers bind per
installation. (Same philosophy as `GRAFFY_MODEL_*`: specs say what, hosts
say how.)

## 6. Security defaults

- `effector` role ⇒ approval node injected in the facade by default
- every invoke journals `args_sha256` + result evidence id — auditable always
- registry pins the server's declared identity; a server whose tool list
  changes shape since install raises a drift warning before the next run
  (MCW Constraint Opacity, applied to tooling)

## 7. MCW alignment (explicit)

The taxonomy is not merely compatible with the MCW Framework — each piece
implements a named construct (@rainmana asked for this to be explicit):

| Design element | MCW construct it implements |
| --- | --- |
| `digest` node preserving schema-salient distinctions | anti-**Overcompression** — summaries must not destroy distinctions needed later |
| `evidence` role feeding `ground` before any draft | the **epistemic floor** — no claim above the evidence layer supporting it |
| `effector` role approval-gated by default | prevents **Asymmetric State Advancement** — the world must not change without the human's externalized consent |
| tool-shape drift warning at run start | **Constraint Opacity** detection applied to tooling |
| the usage interview itself | proactive constraint legibility — surfacing hidden variables *before* they shape behavior |
| interview follow-ups on ambiguous answers | the **Disambiguation** repair operation, used preventively |

## 8. The interview is MCW-instrumented

Each question elicits typed Information Units, and answers are screened for
early failure-mode signals — with targeted follow-ups (budgeted, visit-capped,
because the interview is a graph like everything else):

- Q1 (*what do you use it for?*) elicits **GOAL** and capability IUs.
- Q2 (*does it change things or just look them up?*) elicits **CONSTRAINT**
  IUs. If the answer contradicts the server's own annotations (user says
  "read-only," server declares `destructiveHint`), that is a **False
  Alignment** early signal → one clarifying follow-up, and the conservative
  annotation wins until resolved.
- Q3 (*should graphs reach for it automatically?*) elicits **PRIORITY** IUs.
- Vague answers ("it kind of does stuff") are an **ambiguity signal** → a
  **Disambiguation** follow-up splits the interpretations explicitly rather
  than letting the graph guess.
- Unstated assumptions the agent can detect from context (e.g. credentials
  imply write access) are **Constraint Opacity** probes: asked once, plainly.

Follow-ups cap at two per question — self-correction, not interrogation.

## 9. Future: graffy administering graffy (recorded, not scheduled)

Once the facade machinery exists, graffy's own operations become candidates
for graphification: `mcp add` (discovery → interview → facade generation →
registry write) is itself a graph with an approval gate before the registry
mutates; skill import likewise. The harness eventually administers itself
through the same lawful, journaled machinery it applies to everything else —
"automagical, but inspectable" (@rainmana). Tabled until the primitives are
proven on external servers first.

## 10. Open questions (for review)

1. Facade granularity: per-tool vs per-server-per-role — start per-tool,
   merge later if spec noise is real?
2. Should `digest` be mandatory for `evidence`-role facades (IU discipline)
   and optional for `effector` (result is often just a receipt)? Leaning yes.
3. MCP prompts-primitive import: expose as runnable mini-graphs, or only as
   prepare-node knowledge? Leaning both, prompts-as-graphs being the natural
   graffification of the primitive.
4. Elicitation/sampling (server-initiated requests): map onto
   `PauseForDisambiguation` and the approval machinery — needs a spike.
