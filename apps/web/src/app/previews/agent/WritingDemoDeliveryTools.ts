import type { WritingSubmissionAdmission } from "../../../core/features/agent/writingSubmissionController";
import type { WritingDeliveryToolGateway, WritingDeliveryToolRequest, WritingDeliveryToolResult } from "../../../core/public/agentFileDelivery";

/** Mock the native scope-bound event/waiter transport, never file application. */
export class WritingDemoDeliveryTools implements WritingDeliveryToolGateway {
  constructor(private readonly binding: (admission: WritingSubmissionAdmission) => Promise<WritingDeliveryToolRequest["binding"]>) {}
  private readonly receivers = new Set<(request: WritingDeliveryToolRequest) => void>();
  private readonly pending = new Map<string, { request: WritingDeliveryToolRequest; resolve: (result: WritingDeliveryToolResult) => void }>();
  get connected(): boolean { return this.receivers.size > 0; }
  readonly listen: WritingDeliveryToolGateway["listen"] = async receive => {
    this.receivers.add(receive); return () => { this.receivers.delete(receive); };
  };
  async request(admission: WritingSubmissionAdmission): Promise<WritingDeliveryToolResult> {
    const request = { ...structuredClone(admission), requestId: crypto.randomUUID(), binding: await this.binding(admission) };
    return new Promise(resolve => {
      this.pending.set(request.requestId, { request, resolve });
      for (const receive of this.receivers) receive(request);
    });
  }
  readonly report: WritingDeliveryToolGateway["report"] = async result => {
    const entry = this.pending.get(result.request.requestId);
    if (!entry || JSON.stringify(entry.request) !== JSON.stringify(result.request)) throw new Error("模拟交付工具身份已改变");
    this.pending.delete(result.request.requestId); entry.resolve(structuredClone(result));
  };
  dispose(): void {
    for (const { request, resolve } of this.pending.values()) resolve({ request, complete: false, message: "交付工具已中断；原操作结果仍需核对。" });
    this.pending.clear(); this.receivers.clear();
  }
}
