import { appStorageScopedKey } from "../../runtime/appStorage";
import { forkScope, type ForkScope } from "./conversationForkPending";
import { assertConversationOperationStorageCurrent, browserConversationOperationLock, type ConversationOperationLock } from "./conversationOperationLock";

export type ConversationForkLock = ConversationOperationLock;
/** Standard scoped-release lock. Never steal, queue a stale UI action, or
 * silently fall back to an in-memory/unlocked writer in another context. */
export const browserConversationForkLock: ConversationForkLock = browserConversationOperationLock("FORK_RECOVERY");
export function forkLockName(storage: Storage, scope: ForkScope): string {
  return appStorageScopedKey(storage, `research-assistant:conversation-fork-lock:v1:${forkScope(scope)}`);
}
export async function assertForkStorageCurrent(storage: Storage): Promise<void> {
  await assertConversationOperationStorageCurrent(storage, "FORK_RECOVERY");
}
