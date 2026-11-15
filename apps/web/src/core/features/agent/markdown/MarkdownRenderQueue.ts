import type { MarkdownBlock, MarkdownInput, MarkdownReply, MarkdownRequest } from "./markdownProtocol";

export interface MarkdownWorkerPort {
  postMessage(request: MarkdownRequest): void;
  terminate(): void;
  onmessage: ((event: MessageEvent<MarkdownReply>) => void) | null;
  onerror: ((event: Event) => void) | null;
  onmessageerror: ((event: MessageEvent) => void) | null;
}
export interface MarkdownSnapshot extends MarkdownInput { readonly blocks: readonly MarkdownBlock[]; readonly failed: boolean; }
interface Client {
  readonly receive: (snapshot: MarkdownSnapshot) => void;
  latest: MarkdownInput | null;
  blocks: readonly MarkdownBlock[];
  revision: number;
}
const REFRESH_MS = 80;

/** One queue per transcript. Receive every source update, but retain only the
 * latest pending snapshot per message and one in-flight worker request. Input
 * events and durable Agent event acceptance never pass through this queue. */
export class MarkdownRenderQueue {
  private worker: MarkdownWorkerPort | null = null;
  private readonly clients = new Map<number, Client>();
  private readonly pending = new Map<number, number>();
  private nextOwner = 0;
  private flight: {owner: number; revision: number; input: MarkdownInput} | null = null;
  private timer: ReturnType<typeof setTimeout> | undefined;
  private lastSent = -Infinity;
  private broken = false;
  constructor(private readonly createWorker: () => MarkdownWorkerPort) {}

  connect(receive: Client["receive"]): {update(input: MarkdownInput): void; dispose(): void} {
    const owner = ++this.nextOwner;
    const client: Client = {receive, latest: null, blocks: [], revision: 0};
    this.clients.set(owner, client);
    return {
      update: input => {
        if (!this.clients.has(owner)) return;
        if (client.latest?.value === input.value && client.latest.reveal === input.reveal && client.latest.streaming === input.streaming) return;
        client.latest = input; client.revision++;
        if (this.broken) {receive({...input, blocks: [], failed: true}); return;}
        this.pending.set(owner, client.revision);
        // Finish/cancel/replacement does not wait for the streaming timer.
        this.schedule(!input.streaming);
      },
      dispose: () => {
        this.clients.delete(owner); this.pending.delete(owner);
        this.worker?.postMessage({kind: "release", owner});
        if (!this.clients.size) this.stop();
      },
    };
  }
  private schedule(immediate = false): void {
    if (this.flight || !this.pending.size || this.broken) return;
    if (this.timer !== undefined) {
      if (!immediate) return;
      clearTimeout(this.timer); this.timer = undefined;
    }
    const delay = immediate ? 0 : Math.max(0, REFRESH_MS - (performance.now() - this.lastSent));
    if (delay === 0) this.send();
    else this.timer = setTimeout(() => {this.timer = undefined; this.send();}, delay);
  }
  private send(): void {
    const entry = this.pending.entries().next().value as [number, number] | undefined;
    if (!entry) return;
    const [owner, revision] = entry, client = this.clients.get(owner);
    this.pending.delete(owner);
    if (!client?.latest) {this.schedule(); return;}
    try {
      if (!this.worker) {
        this.worker = this.createWorker();
        this.worker.onmessage = ({data}) => this.received(data);
        this.worker.onerror = this.worker.onmessageerror = () => this.fail();
      }
      this.flight = {owner, revision, input: client.latest};
      this.lastSent = performance.now();
      this.worker.postMessage({kind: "render", owner, revision, input: client.latest});
    } catch {this.fail();}
  }
  private received(reply: MarkdownReply): void {
    const flight = this.flight;
    if (!flight || reply.owner !== flight.owner || reply.revision !== flight.revision) return;
    this.flight = null;
    const client = this.clients.get(reply.owner);
    if (client) {
      if (reply.kind === "rendered") {
        if (reply.blocks.some((patch, index) => !patch.tree && client.blocks[index]?.revision !== patch.revision)) {
          this.fail(); return;
        }
        client.blocks = reply.blocks.map((patch, index) => patch.tree
          ? {revision: patch.revision, tree: patch.tree} : client.blocks[index]!);
      }
      // Appends may display an intermediate accepted snapshot. A replacement
      // or a final received during compilation must never show an older result.
      const latest = client.latest!;
      if (latest.reveal === flight.input.reveal && (client.revision === flight.revision
        || latest.streaming && latest.value.startsWith(flight.input.value))) {
        client.receive({...flight.input, blocks: client.blocks, failed: reply.kind === "failed"});
      }
    }
    const finalWaiting = [...this.pending.keys()].some(owner => !this.clients.get(owner)?.latest?.streaming);
    this.schedule(finalWaiting);
  }
  private fail(): void {
    this.broken = true;
    this.stop();
    for (const client of this.clients.values()) if (client.latest) client.receive({...client.latest, blocks: [], failed: true});
  }
  private stop(): void {
    if (this.timer !== undefined) clearTimeout(this.timer);
    this.timer = undefined; this.flight = null; this.pending.clear();
    if (this.worker) {
      this.worker.onmessage = null; this.worker.onerror = null; this.worker.onmessageerror = null;
      this.worker.terminate(); this.worker = null;
    }
    this.lastSent = -Infinity;
  }
}
