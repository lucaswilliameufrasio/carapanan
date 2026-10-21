# ADR-003: SQLite metadata and local artifact files

- **Status:** Accepted
- **Date:** 2026-10-21
- **Decision owner:** Product owner approval recorded in the implementation conversation
- **Related:** [Development plan](../development-plan.md), [PD-32](../product-decisions.md#pd-32--artifact-store-local-com-quota-e-retenção-explícitas)

## Context

Carapanã needs durable, bounded storage for artifacts without putting large payloads in SQLite rows,
session events, logs, or provider context. The store must remain local-first, preserve durable user work,
survive process interruption without treating partial writes as successful, and make disk usage and
cleanup policy explicit. The feature is not yet authorized for runtime/tool wiring; content safety,
authorization, and sharing gates remain independent.

## Decision

- Keep the catalog and lifecycle metadata in the per-user SQLite database. Store payload bytes in a
  dedicated, private per-user artifact directory with owner-only files and directories.
- Use generated numeric IDs with no embedded source metadata for file names; IDs are identifiers, not
  authorization capabilities. Keep source paths, display names, prompts, and payload bytes out of the
  catalog and logs. Do not compress artifacts.
- Enforce an adjustable initial per-user quota of 10 GiB and a 256 MiB maximum artifact size. Reserve
  capacity before consuming/writing payload bytes; stream with a hard byte limit; reject over-limit
  writes without evicting durable artifacts.
- Retain durable artifacts until explicit deletion. Expire temporary artifacts after seven days and
  clean only store-managed pending/orphan files after a seven-day grace period, in bounded batches.
  Advisory file locks coordinate active readers/writers with recovery and deletion. Missing/corrupt
  durable content remains reported rather than silently dropping its metadata.
- Keep the API local and isolated from tools, provider, daemon, session queue, and UI. Callers remain
  responsible for passing content only after separate secret/content and authorization gates.

## Alternatives considered

1. **Store payloads as SQLite BLOBs.** Rejected because large payloads would inflate the database and
   WAL, couple streaming I/O to transactions, and make existing database backup/checkpoint behavior
   harder to bound.
2. **Use a content-addressed global cache with LRU eviction.** Rejected because automatic eviction
   could delete durable evidence or user work, and content deduplication adds cross-artifact lifecycle
   and privacy complexity before a runtime consumer exists.
3. **Store files without a SQLite catalog.** Rejected because listings, quota reservation, lifecycle
   state, and explicit deletion need transactional metadata and recovery visibility.

## Consequences

- SQLite transactions and filesystem operations cannot be committed atomically together. Writes need explicit
  pending/ready/deleting states, bounded reconciliation, and honest errors for uncertain outcomes.
- The 10 GiB and 256 MiB defaults are initial admission ceilings, not hardware-calibrated targets.
  Benchmarks may justify lower values; increasing either ceiling requires revisiting this decision.
- The store does not classify secrets. No caller may infer that locally persisted content is safe to
  display, log, upload, or send to a provider.
- Durable artifacts can exhaust their quota until the user explicitly deletes some; preserving work is
  preferred to silently reclaiming space.

## Revisit when

Revisit before wiring tool output, checkpoints, validation evidence, or provider context into the store;
before adding automatic durable-artifact deletion; and after storage benchmarks on supported modest
hardware provide evidence for different quota defaults.
