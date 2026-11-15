import type { DocumentCommit, DocumentPort, DocumentSnapshot } from "@research-assistant/core/hbs";
/** Delivery scheduling decorates an actual transport. It cannot synthesize
 * receipts or bypass the authority's validation and transaction. */
export class HbsDeliveryPort<D> implements DocumentPort<D> {
  private readonly listeners = new Set<() => void>();
  private readonly waiting: { request: DocumentCommit<D>; resolve: (value: DocumentSnapshot<D>) => void; reject: (reason: unknown) => void }[] = [];
  private state = { automatic: true, waiting: 0, events: [] as readonly string[] };
  private disposed = false;
  private readonly active = new Set<Promise<void>>();
  constructor(private readonly actual: DocumentPort<D>, private readonly destination = "权威存储") {}
  readonly getSnapshot = () => this.state;
  readonly subscribe = (listener: () => void) => { this.listeners.add(listener); return () => { this.listeners.delete(listener); }; };
  private log(event: string): void { this.state = { ...this.state, waiting: this.waiting.length, events: [...this.state.events, event] }; this.listeners.forEach(listener => listener()); }
  read(): Promise<DocumentSnapshot<D>> { this.log("读取最新 S/B"); return this.actual.read(); }
  commit(request: DocumentCommit<D>): Promise<DocumentSnapshot<D>> {
    if (this.disposed) return Promise.reject(new Error("MODEL_SESSION_CLOSED：旧会话已关闭，不能提交新的写入。"));
    return new Promise((resolve, reject) => {
      this.waiting.push({ request, resolve, reject }); this.log(`排队 ${request.id} · 基于 v${request.expectedVersion}`);
      if (this.state.automatic) void this.deliver();
    });
  }
  async deliver(): Promise<void> {
    if (this.disposed) return;
    const item = this.waiting.shift();
    if (!item) { this.log("通道为空，没有待发送请求"); return; }
    this.log(`发送 ${item.request.id} → ${this.destination}`);
    const operation = (async () => {
      try { const value = await this.actual.commit(item.request); this.log(`回执 ${item.request.id} · v${value.version}`); item.resolve(value); }
      catch (reason) { this.log(`${item.request.id}：${reason instanceof Error ? reason.message : String(reason)}`); item.reject(reason); }
    })();
    this.active.add(operation);
    try { await operation; } finally { this.active.delete(operation); }
  }
  setAutomatic(automatic: boolean): void {
    if (this.disposed) return;
    this.state = { ...this.state, automatic }; this.log(automatic ? "自动发送已开启" : "手动发送已开启");
    if (automatic) while (this.waiting.length) void this.deliver();
  }
  dispose(): void {
    this.disposed = true;
    for (const item of this.waiting.splice(0)) item.reject(new Error("MODEL_SESSION_CLOSED：本地输入保全，未发送的请求已终止。"));
    this.listeners.clear();
  }
  /** Disposal blocks new requests; already dispatched writes must still settle
   * before an owner can replace or clear the resource. */
  async settle(): Promise<void> { await Promise.all([...this.active]); }
}
