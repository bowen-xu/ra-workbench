/** @vitest-environment happy-dom */
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { BoundConversationList } from "./BoundConversationList";

it("loads the next page when the list sentinel approaches the viewport", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  let intersect: IntersectionObserverCallback | undefined;
  vi.stubGlobal("IntersectionObserver", class {
    constructor(callback: IntersectionObserverCallback) {intersect = callback;}
    observe() {} disconnect() {}
  });
  const first = {id: "00000000-0000-4000-8000-000000000001", title: "第一页", archived: false, durableCount: 0, createdAt: "2026-10-01T00:00:00.000Z", updatedAt: "2026-10-01T00:00:00.000Z"};
  const second = {...first, id: "00000000-0000-4000-8000-000000000002", title: "第二页"};
  const cursor = {id: first.id, createdAt: first.createdAt};
  const list = vi.fn(async (request: {before: unknown}) => request.before
    ? {schemaVersion: 1 as const, conversations: [second], next: null}
    : {schemaVersion: 1 as const, conversations: [first], next: cursor});
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  try {
    await act(async () => root.render(<BoundConversationList binding={null} selectedId={null} onSelect={() => {}} catalog={{list, rename: vi.fn(), renameStatus: vi.fn()}} />));
    expect(host.textContent).toContain("第一页");
    expect(intersect).toBeDefined();
    await act(async () => intersect!([{isIntersecting: true} as IntersectionObserverEntry], {} as IntersectionObserver));
    expect(list).toHaveBeenCalledTimes(2);
    expect(host.textContent).toContain("第二页");
  } finally {await act(async () => root.unmount()); host.remove(); vi.restoreAllMocks(); vi.unstubAllGlobals();}
});
