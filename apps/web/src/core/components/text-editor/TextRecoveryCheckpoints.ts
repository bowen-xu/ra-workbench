interface RecoveryState {
  readonly content: string;
  readonly baseline: string;
  readonly saved: string;
  readonly savedBaseline: string;
}
interface Entry {readonly state: RecoveryState; readonly serialized: string}

/** One text workspace. At most 64 entries / 2M UTF-16 units of serialized data. */
export class TextRecoveryCheckpoints {
  private owner: Storage | undefined;
  private readonly entries = new Map<string, Entry>();
  private characters = 0;
  constructor(private readonly storage: () => Storage) {}

  write(key: string, state: RecoveryState): void {
    const storage = this.storage();
    if (storage !== this.owner) {this.entries.clear(); this.characters = 0; this.owner = storage;}
    const before = this.entries.get(key);
    const same = before && before.state.content === state.content && before.state.baseline === state.baseline
      && before.state.saved === state.saved && before.state.savedBaseline === state.savedBaseline;
    const payload = {content: state.content, baseline: state.baseline, saved: state.saved, savedBaseline: state.savedBaseline};
    const serialized = same ? before.serialized : JSON.stringify(payload);
    // The cache does not prove durability. Recheck the storage projection every
    // time; asynchronous close/save guards also await the backend receipt.
    if (storage.getItem(key) !== serialized) storage.setItem(key, serialized);
    this.remove(key);
    this.entries.set(key, {state: payload, serialized}); this.characters += serialized.length;
    while (this.entries.size > 64 || this.characters > 2_000_000) this.remove(this.entries.keys().next().value!);
  }
  retain(keys: readonly string[]): void {
    const retained = new Set(keys);
    for (const key of this.entries.keys()) if (!retained.has(key)) this.remove(key);
  }
  private remove(key: string): void {
    const before = this.entries.get(key);
    if (before) {this.characters -= before.serialized.length; this.entries.delete(key);}
  }
}
