import {afterEach, beforeEach, expect, it, vi} from "vitest";
import {MarkdownRenderQueue, type MarkdownWorkerPort} from "./MarkdownRenderQueue";
import type {MarkdownReply, MarkdownRequest} from "./markdownProtocol";
let port: MarkdownWorkerPort, queue: MarkdownRenderQueue, sent: MarkdownRequest[];
const input = (value: string, streaming = true) => ({value, streaming, reveal: true});
function reply(index: number, tree = true): void {
  const request = sent[index]!;
  if (request.kind !== "render") throw new Error("expected render");
  port.onmessage?.({data: {kind: "rendered", owner: request.owner, revision: request.revision,
    blocks: [{revision: index + 1, ...(tree ? {tree: {type: "text", value: request.input.value}} : {})}]} as MarkdownReply} as MessageEvent<MarkdownReply>);
}
beforeEach(() => {
  vi.useFakeTimers(); sent = [];
  port = {onmessage: null, onerror: null, onmessageerror: null, postMessage: request => sent.push(request), terminate: vi.fn()};
  queue = new MarkdownRenderQueue(() => port);
});
afterEach(() => {vi.useRealTimers();});
it("coalesces a burst into one latest pending snapshot without losing the final source", () => {
  const receive = vi.fn(), client = queue.connect(receive);
  client.update(input("a"));
  for (let i = 0; i < 1000; i++) client.update(input(`a${i}`));
  expect(sent).toHaveLength(1);
  reply(0); expect(receive).toHaveBeenCalledOnce();
  vi.advanceTimersByTime(79); expect(sent).toHaveLength(1);
  vi.advanceTimersByTime(1); expect(sent).toHaveLength(2);
  expect(sent[1]).toMatchObject({input: {value: "a999"}});
  client.update(input("a999 final", false));
  reply(1); expect(receive).toHaveBeenCalledOnce();
  expect(sent).toHaveLength(3); // final does not wait another 80ms
  reply(2); expect(receive.mock.lastCall?.[0].value).toBe("a999 final");
  client.dispose(); expect(port.terminate).toHaveBeenCalledOnce();
});
it("ignores replaced content and obsolete or foreign worker replies", () => {
  const receive = vi.fn(), client = queue.connect(receive);
  client.update(input("old")); client.update(input("replacement", false));
  port.onmessage?.({data: {kind: "failed", owner: 9, revision: 99, code: "AGENT_MARKDOWN_RENDER_FAILED"}} as MessageEvent<MarkdownReply>);
  expect(receive).not.toHaveBeenCalled();
  reply(0); expect(receive).not.toHaveBeenCalled();
  reply(1); expect(receive.mock.lastCall?.[0].value).toBe("replacement");
  client.dispose();
});
it("services multiple owners fairly, releases caches and cancels queued work on unmount", () => {
  const first = vi.fn(), second = vi.fn();
  const a = queue.connect(first), b = queue.connect(second);
  a.update(input("a")); b.update(input("b")); a.update(input("a tail"));
  reply(0); vi.advanceTimersByTime(80);
  expect(sent[1]).toMatchObject({input: {value: "b"}});
  b.dispose(); reply(1); expect(second).not.toHaveBeenCalled();
  a.dispose(); vi.runAllTimers();
  expect(sent.filter(request => request.kind === "render")).toHaveLength(2);
  expect(port.terminate).toHaveBeenCalledOnce();
});
it("retains exact plain text on worker failure instead of parsing synchronously", () => {
  const receive = vi.fn(), client = queue.connect(receive);
  client.update(input("unparsed **text**")); port.onerror?.(new Event("error"));
  expect(receive.mock.lastCall?.[0]).toMatchObject({value: "unparsed **text**", failed: true});
  client.update(input("new content"));
  expect(receive.mock.lastCall?.[0].value).toBe("new content");
  expect(port.terminate).toHaveBeenCalledOnce(); client.dispose();
});
it("preserves block identity across an unchanged worker patch", () => {
  const receive = vi.fn(), client = queue.connect(receive);
  client.update(input("first")); reply(0);
  const block = receive.mock.lastCall![0].blocks[0];
  client.update(input("first next", false));
  const request = sent[1]!;
  if (request.kind !== "render") throw new Error("expected render");
  port.onmessage?.({data: {kind: "rendered", owner: request.owner, revision: request.revision,
    blocks: [{revision: block.revision}]} as MarkdownReply} as MessageEvent<MarkdownReply>);
  expect(receive.mock.lastCall![0].blocks[0]).toBe(block);
  client.dispose();
});
it("fails safely rather than throwing when a block reuse reference has no matching cache", () => {
  const receive = vi.fn(), client = queue.connect(receive);
  client.update(input("exact text"));
  expect(() => reply(0, false)).not.toThrow();
  expect(receive.mock.lastCall?.[0]).toMatchObject({failed: true, value: "exact text"});
  expect(port.terminate).toHaveBeenCalledOnce(); client.dispose();
});
