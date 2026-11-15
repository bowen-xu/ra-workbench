export interface NativeCommandObservation {
  readonly sequence: number;
  readonly command: string;
  readonly startedAtMs: number;
  readonly status: "pending" | "succeeded" | "failed";
  readonly durationMs?: number;
}

/** Bounded, in-memory metadata only. Arguments, results and errors never enter it. */
export class NativeCommandDiagnostics {
  private readonly entries: NativeCommandObservation[] = [];
  private dropped = 0;
  constructor(private readonly clock = () => performance.now(), private readonly limit = 2000) {}

  observe<T>(command: string, operation: () => Promise<T>): Promise<T> {
    if (this.entries.length >= this.limit) { this.dropped++; return operation(); }
    let index: number;
    try {
      index = this.entries.length;
      this.entries.push({sequence: index + 1,
        command: /^[\w:|.-]{1,160}$/.test(command) ? command : "invalid-command-name",
        startedAtMs: this.clock(), status: "pending"});
    } catch { return operation(); }
    const settle = (status: "succeeded" | "failed") => {
      try {
        const before = this.entries[index]!;
        this.entries[index] = {...before, status, durationMs: this.clock() - before.startedAtMs};
      } catch { /* Observation cannot change command settlement. */ }
    };
    try {
      const pending = operation();
      void pending.then(() => settle("succeeded"), () => settle("failed"));
      return pending;
    } catch (reason) { settle("failed"); throw reason; }
  }

  read(): {readonly dropped: number; readonly entries: readonly NativeCommandObservation[]} {
    return {dropped: this.dropped, entries: this.entries.map(entry => ({...entry}))};
  }
}

export const nativeCommandDiagnostics = import.meta.env?.VITE_NATIVE_COMMAND_TRACE === "1"
  ? new NativeCommandDiagnostics() : undefined;

// The opt-in diagnostic surface exposes snapshots, not a transport or write API.
if (nativeCommandDiagnostics && typeof window !== "undefined") {
  try {
    Object.defineProperty(window, "__nativeCommandDiagnostics", {configurable: true,
      value: Object.freeze({read: () => nativeCommandDiagnostics.read()})});
  } catch { /* An unavailable inspector surface must not block native operations. */ }
}
