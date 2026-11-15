import type { ConversationForkLock } from "./conversationForkLock";

/** External-effect fixture; share it between actors to model origin-wide locks. */
export function createTestConversationForkLock(): ConversationForkLock {
  const held = new Set<string>();
  return { async run(name, operation) {
    if (held.has(name)) throw new Error("FORK_RECOVERY_LOCK_BUSY");
    held.add(name);
    try { return await operation(); } finally { held.delete(name); }
  } };
}
