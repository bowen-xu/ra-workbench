/** Presentation of a durable queue claim; this never changes its ownership. */
export type QueuePreparingPresentation = "none" | "sending" | "sent" | "recovery";

interface WritingOwner {
  readonly phase: string;
  readonly error: unknown;
  readonly preparation?: { readonly phase: string; readonly recovered: boolean; readonly intent: { readonly inputId: string } | null } | undefined;
  readonly startRecovery?: { readonly phase: string; readonly recovered: boolean; readonly turnId: string | null;
    readonly intent: { readonly inputId: string; readonly start: { readonly conversationId: string; readonly requestId: string } } | null;
    readonly result: { readonly conversationId: string; readonly requestId: string; readonly turnId: string } | null } | undefined;
}

/** Only a live owner of the exact queue input can make an in-flight claim normal. */
export function writingQueuePreparingPresentation(inputId: string | null | undefined, owner: WritingOwner | null,
  activeDispatchInputId?: string | null): QueuePreparingPresentation {
  if (!inputId) return "none";
  if (activeDispatchInputId === inputId && (!owner || (owner.phase === "idle" && !owner.error
    && !owner.preparation?.intent && !owner.startRecovery?.intent))) return "sending";
  const preparation = owner?.preparation, start = owner?.startRecovery;
  const receipt = start?.result, original = start?.intent;
  if (owner?.phase === "started" && !owner.error && original?.inputId === inputId && receipt && start.turnId === receipt.turnId
    && original.start.conversationId === receipt.conversationId && original.start.requestId === receipt.requestId) return "sent";
  if (owner && !owner.error && (owner.phase === "preparing" || owner.phase === "preview" || owner.phase === "starting")) {
    if (start?.intent?.inputId === inputId && !start.recovered && ["saving", "sending", "clearing"].includes(start.phase)) return "sending";
    if (preparation?.intent?.inputId === inputId && !preparation.recovered && ["persisting", "sending", "ready", "clearing", "complete"].includes(preparation.phase)) return "sending";
  }
  return "recovery";
}

interface IndependentOwner {
  readonly phase: string;
  readonly record: { readonly intent: { readonly queue: { readonly inputId: string } | null };
    readonly committed: { readonly result: { readonly turnId: string } } | null;
    readonly start?: { readonly request: { readonly conversationId: string; readonly requestId: string; readonly queue: { readonly inputId: string } | null };
      readonly result: { readonly conversationId: string; readonly requestId: string; readonly turnId: string } | null } | null } | null;
}

export function independentQueuePreparingPresentation(inputId: string | null | undefined, owner: IndependentOwner | undefined, busy: boolean,
  activeDispatchInputId?: string | null): QueuePreparingPresentation {
  if (!inputId) return "none";
  // The queue claim may arrive before the owner snapshot of the same live
  // dispatch. Do not treat a known in-flight input as a recovered operation.
  if (busy && activeDispatchInputId === inputId && (!owner || (owner.phase === "idle" && !owner.record))) return "sending";
  if (owner?.record?.intent.queue?.inputId !== inputId) return "recovery";
  const start = owner.record.start, receipt = start?.result;
  if (owner.phase === "started" && receipt && start.request.queue?.inputId === inputId && start.request.conversationId === receipt.conversationId
    && start.request.requestId === receipt.requestId && owner.record.committed?.result.turnId === receipt.turnId) return "sent";
  if (busy && (["saving", "sending", "starting"].includes(owner.phase)
    || (owner.phase === "ready" && activeDispatchInputId === inputId))) return "sending";
  return "recovery";
}
