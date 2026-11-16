# ADR-0013: Relicense graffy to Apache-2.0

- Status: **Accepted**
- Date: 2026-11-14 (LICENSE replaced by the owner); recorded 2026-11-16
- Decider: W. Alec Akin (@rainmana)
- Supersedes: ADR-0009

## Decision

graffy is licensed **Apache-2.0** from commit `bf41b68` ("Updating license")
onward. The owner made this change deliberately. This ADR aligns the rest of the
repository with it and replaces ADR-0009's GPL-based inflow rules.

Releases up to and including `v0.1.0-alpha.6` were published under
GPL-3.0-or-later. Those copies keep the terms they shipped with; the change
applies to the project going forward.

Every commit in the repository to date is authored by the owner or by AI
coding agents working at his direction; there are no outside contributors
whose agreement is needed. (This is a statement of the repository history, not
legal advice.)

## What changed with it

- `Cargo.toml` package metadata, README badge, `CONTRIBUTING.md`, `NOTICE.md`,
  `AGENT.md`, `CLAUDE.md`, the four built-in graph specs, and the SPDX headers
  in `src/protos/*.proto`.
- Graphs that graffy **generates** (graphify output, MCP facades) are stamped
  `Apache-2.0` in their `license` field instead of GPL. A user may change that
  field: graph specs they author are their own data.

## Inflow rules (replace ADR-0009's)

- Code and concepts may flow in only from sources whose license is compatible
  with redistribution under Apache-2.0: Apache-2.0, MIT, BSD, ISC, Zlib,
  Unicode and similar permissive terms.
- **GPL, AGPL and LGPL-only code does not enter the tree.** Under ADR-0009 the
  flow was one-way *into* a GPL project; that direction is now reversed, so
  sources that were acceptable before must be re-checked rather than assumed.
- Weak, file-level copyleft (MPL-2.0) is acceptable for an **unmodified
  dependency**, not for vendored or modified source.
- Every inflow — dependency of note, ported concept, or vendored code — is
  still recorded in `NOTICE.md`; vendored code keeps its upstream headers.
- Unlicensed snippets do not enter the tree.

## Verification (2026-11-16)

`cargo metadata --locked` over the workspace: 445 third-party packages. None is
GPL/AGPL-only. One, `option-ext`, is MPL-2.0 and is used unmodified. Two
(`r-efi`) and one (`termina`) are multi-licensed with a permissive option. All
others are permissive. This check reads declared crate metadata; it is not a
legal audit of each crate's contents.

## Consequences

- The owner's **friendo** project is GPL-3.0-only. Its code cannot be copied
  into graffy as is. Because he wrote it, he can release the parts graffy
  borrows under Apache-2.0; that must be an explicit step, recorded in
  `NOTICE.md`. Ideas and designs are not restricted.
- The MCW Framework (Apache-2.0) and the MIT design references in `NOTICE.md`
  remain compatible.
- Users who ship modified graffy binaries no longer have a source-disclosure
  obligation; Apache-2.0's notice and attribution terms apply instead.
