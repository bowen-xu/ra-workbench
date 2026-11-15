/** One preview-only lost acknowledgment, after the real queue edit commits. */
export class NativePreviewResumeHold {
  private armed: {onCommitted: () => void; wait: Promise<void>; release: () => void; released: boolean; consumed: boolean} | null = null;
  arm(onCommitted: () => void): () => void {
    if (this.armed) throw new Error("CONTEXT_RESUME_HOLD_BUSY");
    let resolve!: () => void;
    const entry = {onCommitted, wait: new Promise<void>(done => {resolve = done;}), release: () => resolve(), released: false, consumed: false};
    this.armed = entry;
    return () => {entry.released = true; entry.release(); if (this.armed === entry) this.armed = null;};
  }
  cancel(): void {const entry = this.armed; if (entry) {entry.released = true; entry.release(); this.armed = null;}}
  async after(command: string, args: Record<string, unknown> | undefined): Promise<void> {
    const request = args?.request;
    if (command !== "agent_edit_context_input_queue" || !request || typeof request !== "object" || !("edit" in request)
      || !request.edit || typeof request.edit !== "object" || !("type" in request.edit) || request.edit.type !== "resume") return;
    const entry = this.armed; if (!entry || entry.consumed) return;
    entry.consumed = true;
    if (entry.released) return;
    entry.onCommitted(); await entry.wait;
    throw {code: "AGENT_PREVIEW_SYNTHETIC_IO_RESUME_ACK_LOST"};
  }
}
