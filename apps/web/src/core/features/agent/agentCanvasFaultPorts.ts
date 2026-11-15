export type CanvasQueueFailure = "rejected" | "beforeCommit" | "afterCommit" | "unavailable";
export type CanvasSteerFailure = "normal" | "rejected" | "beforeCommit" | "afterCommit";
export interface CanvasQueueFaultPort { setNextEditFailure(failure: CanvasQueueFailure | null): void | Promise<void> }
export interface CanvasDeletionFaultPort { setNextDeletionFailure(failure: CanvasQueueFailure | null): void | Promise<void> }
export interface CanvasRecoveryFaultPort {
  setSteerFailure(failure: CanvasSteerFailure): void | Promise<void>;
  disconnect(mode: "recover" | "exhaust"): void | Promise<void>;
}
