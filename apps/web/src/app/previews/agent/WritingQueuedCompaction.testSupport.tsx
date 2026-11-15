import {registerStoragePersistence} from "../../../core/runtime/storagePersistence";
import {selectAppStorage} from "../../../core/runtime/appStorage";
import {resetWritingSubmissionControllersForTests} from "../../../core/features/agent/writingSubmissionController";
import {WritingDiffDemoStorage} from "./WritingDiffDemoStorage";
import {WritingDiffDemoAgentStorage} from "./WritingDiffDemoAgentStorage";
import {createWritingDiffDemoRuntime} from "./writingDiffDemoRuntime";

class MemoryStorage implements Storage {
  readonly values = new Map<string, string>();
  get length() {return this.values.size;}
  key(index: number) {return [...this.values.keys()][index] ?? null;}
  getItem(key: string) {return this.values.get(key) ?? null;}
  setItem(key: string, value: string) {this.values.set(key, value);}
  removeItem(key: string) {this.values.delete(key);}
  clear() {this.values.clear();}
}

/** Application-owned external I/O and cold-process composition for domain
 * tests. The boundary guard admits this entry only from static test imports. */
export function createQueuedCompactionTestRuntime(): {
  readonly storage: MemoryStorage;
  readonly scope: {readonly manuscriptId: string; readonly conversationId: string};
  readonly runtime: ReturnType<typeof createWritingDiffDemoRuntime>;
} {
  const storage = new MemoryStorage(); selectAppStorage(storage);
  const scope = {manuscriptId: crypto.randomUUID(), conversationId: crypto.randomUUID()};
  const files = new WritingDiffDemoStorage(scope.manuscriptId, []), agent = new WritingDiffDemoAgentStorage(files, undefined);
  const runtime = createWritingDiffDemoRuntime(files, agent, scope.manuscriptId, scope.conversationId);
  return {storage, scope, runtime};
}
export function resetQueuedCompactionTestOwners(): void {resetWritingSubmissionControllersForTests();}
export function queuedCompactionTestPersistence(storage: Storage, flush: () => Promise<void>): void {
  registerStoragePersistence(storage, {backend: "IndexedDB", pending: false, resetting: false, error: undefined,
    flush, retry: async () => {}, reset: async () => {}, close: () => {}, subscribe: () => () => {}});
}
