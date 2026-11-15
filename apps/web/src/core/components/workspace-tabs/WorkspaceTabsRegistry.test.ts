import { expect, it, vi } from "vitest";
import { WorkspaceTabsRegistry } from "./WorkspaceTabsRegistry";
import type { WorkspaceTab } from "./WorkspaceTabs";
it("retains saved cross-workspace order while owners mount lazily, and removes explicit closures",()=>{
  const persist=vi.fn(),registry=new WorkspaceTabsRegistry(["note","pdf","tex"],persist);
  const tab=(id:string):WorkspaceTab=>({id,owner:"library",title:id,active:false,select:()=>{}});
  registry.set("pdf",[tab("pdf")]);registry.set("writing",[tab("tex")]);registry.set("notes",[tab("note")]);
  expect(registry.getSnapshot().map(tab=>tab.id)).toEqual(["note","pdf","tex"]);
  registry.set("notes",[]);
  expect(persist.mock.lastCall?.[0]).toEqual(["pdf","tex"]);
});

it("forwards retained select, close and pin actions while publishing only observable changes", () => {
  const registry = new WorkspaceTabsRegistry(), changed = vi.fn(), action = vi.fn();
  registry.subscribe(changed);
  const tab = (version: number): WorkspaceTab => ({id: "file", owner: "writing", title: "main.tex", active: true,
    select: () => action("select", version), close: () => action("close", version), pin: () => action("pin", version)});
  registry.set("writing", [tab(1)]); const retained = registry.getSnapshot();
  registry.set("writing", [tab(2)]);
  expect(registry.getSnapshot()).toBe(retained); expect(changed).toHaveBeenCalledOnce();
  retained[0]!.select(); retained[0]!.close!(); retained[0]!.pin!();
  expect(action.mock.calls).toEqual([["select", 2], ["close", 2], ["pin", 2]]);
  registry.set("writing", [{...tab(3), pending: true}]);
  expect(changed).toHaveBeenCalledTimes(2);
  const {close: _close, ...withoutClose} = tab(4);
  registry.set("writing", [withoutClose]);
  expect(registry.getSnapshot()[0]!.close).toBeUndefined(); expect(changed).toHaveBeenCalledTimes(3);
  registry.remove("missing"); expect(changed).toHaveBeenCalledTimes(3);
  registry.remove("writing"); expect(registry.getSnapshot()).toEqual([]);
  expect(changed).toHaveBeenCalledTimes(4);
});
