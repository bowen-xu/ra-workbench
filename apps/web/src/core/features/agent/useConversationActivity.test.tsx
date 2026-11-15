/** @vitest-environment happy-dom */
import { act, useRef } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import type { AgentCatalogGateway } from "@research-assistant/data/agent";
import { useConversationActivity } from "./useConversationActivity";

function Harness({catalog}: {readonly catalog: AgentCatalogGateway}) {
  const element = useRef<HTMLElement>(null);
  const running = useConversationActivity(catalog, null, element);
  return <section ref={element}>{running.map(item => item.id).join(",")}</section>;
}

it("reads only on semantic invalidations, including a single async-registration recheck", async () => {
  vi.useFakeTimers();
  const order: string[] = [], stop = vi.fn();
  let changed = () => {}, registered!: (stop: () => void) => void;
  let running = [{id: "one", internal: false}];
  const activity = vi.fn(async () => { order.push("read"); return running; });
  const catalog: AgentCatalogGateway = {activity, list: vi.fn(), rename: vi.fn(), renameStatus: vi.fn(),
    subscribeActivity: notify => {
      order.push("subscribe"); changed = notify;
      return new Promise(resolve => { registered = resolve; });
    }};
  const container = document.createElement("div"), root = createRoot(container);
  document.body.append(container);
  try {
    await act(async () => root.render(<Harness catalog={catalog}/>));
    expect(order).toEqual(["subscribe", "read"]);
    expect(container.textContent).toBe("one");
    await act(async () => vi.advanceTimersByTimeAsync(5000));
    expect(activity).toHaveBeenCalledTimes(1);
    running = [{id: "two", internal: false}];
    await act(async () => { registered(stop); });
    expect(activity).toHaveBeenCalledTimes(2);
    expect(container.textContent).toBe("two");
    running = [];
    await act(async () => changed());
    expect(activity).toHaveBeenCalledTimes(3);
    expect(container.textContent).toBe("");
    await act(async () => vi.advanceTimersByTimeAsync(5000));
    expect(activity).toHaveBeenCalledTimes(3);
  } finally { await act(async () => root.unmount()); container.remove(); vi.useRealTimers(); }
  expect(stop).toHaveBeenCalledTimes(1);
  await act(async () => changed());
  expect(activity).toHaveBeenCalledTimes(3);
});

it("releases a listener registered after cleanup without querying again", async () => {
  let registered!: (stop: () => void) => void;
  const stop = vi.fn(), activity = vi.fn(async () => []);
  const catalog: AgentCatalogGateway = {activity, list: vi.fn(), rename: vi.fn(), renameStatus: vi.fn(),
    subscribeActivity: () => new Promise(resolve => { registered = resolve; })};
  const root = createRoot(document.createElement("div"));
  await act(async () => root.render(<Harness catalog={catalog}/>));
  await act(async () => root.unmount());
  await act(async () => { registered(stop); });
  expect(stop).toHaveBeenCalledTimes(1);
  expect(activity).toHaveBeenCalledTimes(1);
});
