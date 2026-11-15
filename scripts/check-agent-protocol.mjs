import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { isDeepStrictEqual } from "node:util";

const actual = execFileSync("cargo", ["run", "--quiet", "--locked", "--manifest-path",
  "packages/agent-core/Cargo.toml", "--example", "export_event_fixture"], { encoding: "utf8" });
const expected = readFileSync("tests/fixtures/agent-events-v1.json", "utf8");
if (actual !== expected) {
  throw new Error("Agent protocol fixture is stale; regenerate it with the Rust export_event_fixture example and review both wire contracts.");
}

// The page belongs to the same complete ledger, including compaction items.
// Updating the event fixture must not leave the page watermark behind.
const events = JSON.parse(actual);
const history = JSON.parse(readFileSync("tests/fixtures/agent-history-v1.json", "utf8"));
const page = events.slice(0, 3);
if (!isDeepStrictEqual(history, {
  schemaVersion: 1,
  conversationId: events[0].conversationId,
  afterSeq: 0,
  events: page,
  watermark: events.at(-1).seq,
  nextAfterSeq: page.at(-1).seq,
})) {
  throw new Error("Agent history fixture is stale; review its page and watermark against the complete event fixture.");
}

// A separate Rust-owned ledger proves rollback without rewriting the original
// event golden. The exporter validates every event against the real projection.
const rollback = execFileSync("cargo", ["run", "--quiet", "--locked", "--manifest-path",
  "packages/agent-core/Cargo.toml", "--example", "export_rollback_fixture"], { encoding: "utf8" });
const expectedRollback = readFileSync("tests/fixtures/agent-rollback-events-v1.json", "utf8");
if (rollback !== expectedRollback) {
  throw new Error("Agent rollback fixture is stale; regenerate it with the Rust export_rollback_fixture example and review its user-turn boundaries.");
}
