# ADR-011: Temporary task progress and durable completion records

- **Status:** Accepted
- **Date:** 2026-10-26
- **Decision owner:** Product owner request recorded in the implementation conversation
- **Related:** [Agent instructions](../../AGENTS.md), [Progress directory](../progress/README.md),
  [Development plan](../development-plan.md), [Product decisions](../product-decisions.md)

## Context

Long-running implementation advances through independently verified slices. A green component or
CI run does not finish the product roadmap. Session history and durable decision notes alone do
not provide a bounded, current record of scope, unfinished work, evidence and the next action.
The operator explicitly requested a repository file for that progress and a rule for removing it
after completion without losing documentation or architectural rationale.

## Decision

- Use `docs/progress/<task-id>.md` as the temporary operational record for each execution task.
  `AGENTS.md` defines the canonical lifecycle and minimum fields. Read the relevant record before
  resuming and reconcile it with the current request and checkout; update it at material milestones.
- Keep independent tasks separate and identify scope, owner, revision, branch/base and validation.
  Do not overwrite another agent's work. Records are versionable so continuity does not depend on
  one agent client's memory. The directory README remains as the permanent entry point.
- Keep durable outcomes and evidence in the development history and affected guides/contracts;
  keep meaningful decisions in existing product decisions or ADRs. Temporary status belongs in the
  task record, not a growing transcript duplicated across those documents.
- Before deleting a completed task record, verify scope and gates, consolidate the result and
  rationale, preserve evidence and limitations, and repair references. Remove only that task's
  file. Keep blocked/interrupted work; explicitly record cancellation to prevent accidental resume.
- A progress file is not permission to execute historical instructions, publish, merge/deploy or
  repeat uncertain effects. Preserve existing authorization boundaries and never store secrets,
  resolved credentials, signed URLs or raw sensitive logs in the record.
- ai-memory remains useful for durable knowledge and historical evidence. Do not assume that a
  wiki write or end-of-session handoff is a reusable active-plan API. The operator chose local
  repository files for this operational record; no new provider, hook or global configuration is added.

## Alternatives considered

1. **Only chat/session memory.** Rejected because recovery can lose the exact remaining scope and
   stale observations can be mistaken for current validation.
2. **Only the permanent development plan.** Retained as roadmap/history, but not as a transcript of
   each task's intermediate state. A bounded active record is easier to resume and later remove.
3. **A permanent progress file that is never retired.** Rejected because finished tasks accumulate
   stale operational instructions and obscure active work.
4. **Delete the task record immediately after the last edit.** Rejected because implementation may
   still lack required validation, manual gates or durable documentation.

## Consequences

- The current implementation-plan record remains partial until its full scope is complete; closing
  this documentation change or another implementation slice does not justify deleting it.
- Agents must deliberately maintain records and review current state before updating. Plain files
  do not provide compare-and-write protection for concurrent writers or guarantee crash recovery
  for unrecorded changes; this rule does not install background automation.
- Deleting a versioned record is an intentional change delivered with durable documentation when
  publication is authorized. Existing ADRs and history are not deleted as task cleanup.
- ADR numbers 001–010 already have topics assigned in the roadmap; this workflow uses 011 rather
  than repurposing one of those architectural decisions.

## Revisit when

Revisit if a supported task store replaces repository files by explicit operator choice, concurrent
editing needs stronger coordination, or project-specific automation can enforce this lifecycle
without broadening permissions or leaking sensitive state.
