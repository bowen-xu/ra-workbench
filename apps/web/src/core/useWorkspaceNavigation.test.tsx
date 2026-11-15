// @vitest-environment happy-dom
import { act, useState } from "react";
import { createRoot } from "react-dom/client";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { loadWorkspaceLocation, useWorkspaceNavigation } from "./useWorkspaceNavigation";
import {writeWorkspaceState} from "./runtime/workspaceState";
import {markNoteDraftIdentity} from "./features/notes/workspace/noteDraftIdentity";
import { selectAppStorage } from "./runtime/appStorage";
import {getAppStorage} from "./runtime/appStorage";

beforeEach(() => {
  const values = new Map<string, string>();
  selectAppStorage({ get length() { return values.size; }, getItem: key => values.get(key) ?? null,
    setItem: (key, value) => { values.set(key, value); }, removeItem: key => { values.delete(key); },
    key: index => [...values.keys()][index] ?? null, clear: () => values.clear() });
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
});

it("keeps an explicit writing destination while its runtime view is still being registered", async () => {
  let navigation!: ReturnType<typeof useWorkspaceNavigation>;
  function Harness({ready}: {readonly ready: boolean}) {
    navigation = useWorkspaceNavigation({view: "writing"}, undefined, {
      availableViews: ready ? ["writing", "settings"] : ["settings"], deferredRestore: !ready,
    });
    return null;
  }
  const root = createRoot(document.createElement("div"));
  try {
    await act(async () => root.render(<Harness ready={false}/>));
    expect(navigation.location).toEqual({view: "writing"});
    await act(async () => root.render(<Harness ready/>));
    expect(navigation.location).toEqual({view: "writing"});
  } finally {await act(async () => root.unmount());}
});

it("defers runtime restore without overwriting the saved source, then restores its original PDF tab once", async () => {
  const original = {view: "library", tabId: "original-pdf"};
  writeWorkspaceState("location", original);
  let navigation!: ReturnType<typeof useWorkspaceNavigation>;
  const restore = {homeTabId: "library", readActiveTabId: vi.fn(() => "session-pdf")};
  function Harness({ready}: {readonly ready: boolean}) {
    navigation = useWorkspaceNavigation(undefined, ready ? restore : undefined,
      {availableViews: ready ? ["library", "settings"] : ["settings"],
        persistenceKey: "location:composition:core", deferredRestore: !ready});
    return null;
  }
  const root = createRoot(document.createElement("div"));
  try {
    await act(async () => root.render(<Harness ready={false}/>));
    expect(navigation.view).toBe("settings");
    expect(loadWorkspaceLocation()).toEqual(original);
    expect(getAppStorage().getItem("research-assistant:workspace.v1:location:composition:core")).toBeNull();
    await act(async () => root.render(<Harness ready/>));
    expect(navigation.location).toEqual(original);
    expect(loadWorkspaceLocation(restore, {availableViews: ["library", "settings"], persistenceKey: "location:composition:core"})).toEqual(original);
    await act(async () => navigation.visitLibraryTab("next-pdf"));
    await act(async () => root.render(<Harness ready/>));
    expect(navigation.location).toEqual({view: "library", tabId: "next-pdf"});
  } finally {await act(async () => root.unmount());}
});

it.each(["writing", "notes"] as const)("restores %s management after runtime registration without requiring a literature provider", async view => {
  const persistenceKey = `location:composition:${view}-only`;
  writeWorkspaceState(persistenceKey, {view});
  let navigation!: ReturnType<typeof useWorkspaceNavigation>;
  function Harness({ready}: {readonly ready: boolean}) {
    navigation = useWorkspaceNavigation(undefined, undefined, {
      availableViews: ready ? [view, "settings"] : ["settings"],
      persistenceKey, deferredRestore: !ready,
    });
    return null;
  }
  const root = createRoot(document.createElement("div"));
  try {
    await act(async () => root.render(<Harness ready={false}/>));
    expect(navigation.view).toBe("settings");
    await act(async () => root.render(<Harness ready/>));
    expect(navigation.location).toEqual({view});
  } finally {await act(async () => root.unmount());}
});

it("waits for the selected runtime views after literature is ready before consuming the saved destination", async () => {
  writeWorkspaceState("location", {view: "writing"});
  const restore = {homeTabId: "library", readActiveTabId: () => "saved-pdf"};
  let navigation!: ReturnType<typeof useWorkspaceNavigation>;
  function Harness({phase}: {readonly phase: "discovering" | "literature" | "ready"}) {
    navigation = useWorkspaceNavigation(undefined, phase === "discovering" ? undefined : restore, {
      availableViews: phase === "ready" ? ["library", "writing", "settings"]
        : phase === "literature" ? ["library", "settings"] : ["settings"],
      deferredRestore: phase !== "ready",
    });
    return null;
  }
  const root = createRoot(document.createElement("div"));
  try {
    await act(async () => root.render(<Harness phase="discovering"/>));
    await act(async () => root.render(<Harness phase="literature"/>));
    expect(navigation.view).toBe("settings");
    expect(loadWorkspaceLocation()).toEqual({view: "writing"});
    await act(async () => root.render(<Harness phase="ready"/>));
    expect(navigation.location).toEqual({view: "writing"});
  } finally {await act(async () => root.unmount());}
});

it("retains a temporarily unavailable runtime destination until it returns or the user chooses the fallback", async () => {
  writeWorkspaceState("location", {view: "custom-fixture"});
  let navigation!: ReturnType<typeof useWorkspaceNavigation>;
  function Harness({available}: {readonly available: boolean}) {
    navigation = useWorkspaceNavigation(undefined, undefined, {
      availableViews: available ? ["custom-fixture", "settings"] : ["settings"],
    });
    return null;
  }
  const root = createRoot(document.createElement("div"));
  try {
    await act(async () => root.render(<Harness available={false}/>));
    expect(navigation.view).toBe("settings");
    expect(JSON.parse(getAppStorage().getItem("research-assistant:workspace.v1:location")!)).toEqual({view: "custom-fixture"});
    await act(async () => root.render(<Harness available/>));
    expect(navigation.view).toBe("custom-fixture");
    await act(async () => root.render(<Harness available={false}/>));
    expect(navigation.view).toBe("settings");
    await act(async () => navigation.navigate("settings"));
    await act(async () => root.render(<Harness available/>));
    expect(navigation.view).toBe("settings");
    expect(JSON.parse(getAppStorage().getItem("research-assistant:workspace.v1:location")!)).toEqual({view: "settings"});
  } finally {await act(async () => root.unmount());}
});

it("keeps a Core document's saved management context while its plugin is unavailable", async () => {
  const original = {view: "notes", notePageId: "saved-page", presentation: "document", workspaceView: "custom-fixture"};
  writeWorkspaceState("location", original);
  let navigation!: ReturnType<typeof useWorkspaceNavigation>;
  function Harness({available}: {readonly available: boolean}) {
    navigation = useWorkspaceNavigation(undefined, undefined, {
      availableViews: available ? ["custom-fixture", "settings"] : ["settings"],
    });
    return null;
  }
  const root = createRoot(document.createElement("div"));
  try {
    await act(async () => root.render(<Harness available={false}/>));
    expect(navigation.notePageId).toBe("saved-page");
    expect(JSON.parse(getAppStorage().getItem("research-assistant:workspace.v1:location")!)).toEqual(original);
    await act(async () => root.render(<Harness available/>));
    expect(navigation.location).toEqual(original);
  } finally {await act(async () => root.unmount());}
});

it("does not let a late source provider take navigation after the user chooses even the current fallback", async () => {
  writeWorkspaceState("location", {view: "library", tabId: "original-pdf"});
  let navigation!: ReturnType<typeof useWorkspaceNavigation>;
  const restore = {homeTabId: "library", readActiveTabId: () => "session-pdf"};
  function Harness({ready}: {readonly ready: boolean}) {
    navigation = useWorkspaceNavigation(undefined, ready ? restore : undefined,
      {availableViews: ready ? ["library", "settings"] : ["settings"], persistenceKey: "location:composition:core", deferredRestore: !ready});
    return null;
  }
  const root = createRoot(document.createElement("div"));
  try {
    await act(async () => root.render(<Harness ready={false}/>));
    await act(async () => navigation.navigate("settings"));
    expect(loadWorkspaceLocation(undefined, {availableViews: ["settings"], persistenceKey: "location:composition:core"})).toEqual({view: "settings"});
    await act(async () => root.render(<Harness ready/>));
    expect(navigation.location).toEqual({view: "settings"});
  } finally {await act(async () => root.unmount());}
});

it("keeps explicit initialLocation ahead of a late source provider", async () => {
  writeWorkspaceState("location", {view: "library", tabId: "saved-pdf"});
  let navigation!: ReturnType<typeof useWorkspaceNavigation>;
  const restore = {homeTabId: "library", readActiveTabId: () => "session-pdf"};
  function Harness({ready}: {readonly ready: boolean}) {
    navigation = useWorkspaceNavigation({view: "notes", notePageId: "explicit-page"}, ready ? restore : undefined,
      {availableViews: ready ? ["library", "notes", "settings"] : ["notes", "settings"], deferredRestore: !ready});
    return null;
  }
  const root = createRoot(document.createElement("div"));
  try {
    await act(async () => root.render(<Harness ready={false}/>));
    await act(async () => root.render(<Harness ready/>));
    expect(navigation.location).toEqual({view: "notes", notePageId: "explicit-page"});
  } finally {await act(async () => root.unmount());}
});

it("drops a vanished empty draft from restored navigation but keeps published pages with the same stable ID",()=>{
  const id=crypto.randomUUID();markNoteDraftIdentity(id,true);
  writeWorkspaceState("location",{view:"notes",notePageId:id});
  writeWorkspaceState("note-tabs",[]);
  expect(loadWorkspaceLocation()).toEqual({view:"notes"});
  markNoteDraftIdentity(id,false);
  expect(loadWorkspaceLocation()).toEqual({view:"notes",notePageId:id});
});

it("uses a composition-local fallback without overwriting the unavailable workspace's restored location, note tabs, or drafts", async () => {
  const location = {view: "writing", tabId: "project-tab"};
  writeWorkspaceState("location", location); writeWorkspaceState("note-tabs", [{id: "canonical-note", title: "Note", preview: false}]);
  getAppStorage().setItem("conversation-draft", "unsent input");
  const before = getAppStorage().getItem("research-assistant:workspace.v1:note-tabs");
  let navigation!: ReturnType<typeof useWorkspaceNavigation>;
  function Harness() {navigation = useWorkspaceNavigation(undefined, undefined, {availableViews: ["settings"], persistenceKey: "location:composition:core"}); return null;}
  const root = createRoot(document.createElement("div"));
  try {
    await act(async () => root.render(<Harness/>));
    expect(navigation.view).toBe("settings");
    expect(loadWorkspaceLocation()).toEqual(location);
    expect(getAppStorage().getItem("research-assistant:workspace.v1:note-tabs")).toBe(before);
    expect(getAppStorage().getItem("conversation-draft")).toBe("unsent input");
    expect(loadWorkspaceLocation(undefined, {availableViews: ["writing", "settings"]})).toEqual(location);
  } finally {await act(async () => root.unmount());}
});

it("restores a registered view only while the caller supplies that stable ID", () => {
  writeWorkspaceState("location", {view: "notes-fixture"});
  expect(loadWorkspaceLocation()).toEqual({view: "library", tabId: "library"});
  expect(loadWorkspaceLocation(undefined, {availableViews: ["notes-fixture", "settings"]})).toEqual({view: "notes-fixture"});
  expect(loadWorkspaceLocation(undefined, {availableViews: ["settings"]})).toEqual({view: "settings"});
});

it("preserves a registered management context when its Core document tab closes", async () => {
  let navigation!: ReturnType<typeof useWorkspaceNavigation>;
  function Harness() {
    navigation = useWorkspaceNavigation({view: "notes-fixture"}, undefined,
      {availableViews: ["notes-fixture", "settings"]});
    return null;
  }
  const root = createRoot(document.createElement("div"));
  try {
    await act(async () => root.render(<Harness/>));
    await act(async () => navigation.navigate("notes", "original-page", "document"));
    expect(navigation.location).toEqual({view: "notes", notePageId: "original-page", presentation: "document", workspaceView: "notes-fixture"});
    expect(loadWorkspaceLocation(undefined, {availableViews: ["notes-fixture", "settings"]})).toEqual(navigation.location);
    await act(async () => navigation.reconcileNotes([], undefined, "workspace"));
    expect(navigation.location).toEqual({view: "notes-fixture"});
    await act(async () => navigation.back());
    expect(navigation.workspaceView).toBe("notes-fixture");
  } finally {await act(async () => root.unmount());}
});

it("removes released views from Back/Forward while preserving their Core note documents", async () => {
  let navigation!: ReturnType<typeof useWorkspaceNavigation>, release!: () => void;
  function Harness() {
    const [available, setAvailable] = useState(["notes", "notes-fixture", "settings"]);
    release = () => setAvailable(["notes", "settings"]);
    navigation = useWorkspaceNavigation({view: "notes", notePageId: "original-page"}, undefined, {availableViews: available});
    return null;
  }
  const root = createRoot(document.createElement("div"));
  try {
    await act(async () => root.render(<Harness/>));
    await act(async () => navigation.navigate("notes-fixture"));
    await act(async () => navigation.navigate("notes", "same-page", "document"));
    await act(async () => navigation.navigate("settings"));
    await act(async () => release());
    await act(async () => navigation.back());
    expect(navigation.location).toEqual({view: "notes", notePageId: "same-page", presentation: "document"});
    await act(async () => navigation.back());
    expect(navigation.location).toEqual({view: "notes", notePageId: "original-page"});
    await act(async () => navigation.forward());
    expect(navigation.location.notePageId).toBe("same-page");
    await act(async () => navigation.forward());
    expect(navigation.location).toEqual({view: "settings"});
  } finally {await act(async () => root.unmount());}
});

it("collapses adjacent visits after releasing the only intervening view", async () => {
  let navigation!: ReturnType<typeof useWorkspaceNavigation>, release!: () => void;
  function Harness() {
    const [available, setAvailable] = useState(["settings", "notes-fixture"]);
    release = () => setAvailable(["settings"]);
    navigation = useWorkspaceNavigation({view: "settings"}, undefined, {availableViews: available});
    return null;
  }
  const root = createRoot(document.createElement("div"));
  try {
    await act(async () => root.render(<Harness/>));
    await act(async () => navigation.navigate("notes-fixture"));
    await act(async () => navigation.navigate("settings"));
    await act(async () => release());
    expect(navigation.location).toEqual({view: "settings"});
    expect(navigation.previous).toBeUndefined();
    expect(navigation.next).toBeUndefined();
  } finally {await act(async () => root.unmount());}
});

describe("workspace visit history", () => {
  async function harness(restore?: Parameters<typeof useWorkspaceNavigation>[1]) {
    (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    let navigation!: ReturnType<typeof useWorkspaceNavigation>;
    function Harness() { navigation = useWorkspaceNavigation(undefined,restore); return null; }
    const root = createRoot(document.createElement("div"));
    await act(async () => root.render(<Harness />));
    return { get navigation() { return navigation; }, root };
  }

  it("traverses list, PDF visits and workspaces without duplicating visits or retaining a new forward branch", async () => {
    const h = await harness();
    try {
      await act(async () => h.navigation.visitLibraryTab("reader:a"));
      await act(async () => h.navigation.visitLibraryTab("reader:b"));
      await act(async () => h.navigation.visitLibraryTab("reader:b"));
      await act(async () => h.navigation.navigate("notes"));
      await act(async () => h.navigation.back());
      expect(h.navigation.location).toEqual({ view: "library", tabId: "reader:b" });
      await act(async () => h.navigation.back());
      expect(h.navigation.location.tabId).toBe("reader:a");
      await act(async () => h.navigation.back());
      expect(h.navigation.location.tabId).toBe("library");
      expect(h.navigation.previous).toBeUndefined();
      await act(async () => h.navigation.forward());
      expect(h.navigation.location.tabId).toBe("reader:a");
      await act(async () => h.navigation.visitLibraryTab("reader:c"));
      expect(h.navigation.next).toBeUndefined();
    } finally { await act(async () => h.root.unmount()); }
  });

  it("removes closed PDF destinations without reopening them or losing the current workspace", async () => {
    const h = await harness();
    try {
      await act(async () => h.navigation.visitLibraryTab("reader:a"));
      await act(async () => h.navigation.visitLibraryTab("reader:b"));
      await act(async () => h.navigation.reconcileLibrary(["reader:a"], "reader:a"));
      expect(h.navigation.location.tabId).toBe("reader:a");
      await act(async () => h.navigation.navigate("settings"));
      await act(async () => h.navigation.reconcileLibrary([], "library"));
      expect(h.navigation.view).toBe("settings");
      await act(async () => h.navigation.back());
      expect(h.navigation.location.tabId).toBe("library");
      expect(h.navigation.previous).toBeUndefined();
    } finally { await act(async () => h.root.unmount()); }
  });

  it("starts at the saved PDF without inheriting navigation history from a previous process", async () => {
    const readActiveTabId = vi.fn(() => "reader:a");
    const h = await harness({homeTabId:"library",readActiveTabId});
    expect(readActiveTabId).toHaveBeenCalledOnce();
    try {
      expect(h.navigation.location.tabId).toBe("reader:a");
      expect(h.navigation.previous).toBeUndefined();
      expect(h.navigation.next).toBeUndefined();
    } finally { await act(async () => h.root.unmount()); }
  });
  it("restores the current workspace and note page after restart without replaying history",async()=>{
    const first=await harness();
    await act(async()=>first.navigation.visitNotePage("note-reopen"));
    await act(async()=>first.root.unmount());
    const second=await harness();
    try {
      expect(second.navigation.location).toEqual({view:"notes",notePageId:"note-reopen"});
      expect(second.navigation.previous).toBeUndefined();expect(second.navigation.next).toBeUndefined();
    } finally {await act(async()=>second.root.unmount());}
  });

  it("restores the last library surface on menu visits, while Home explicitly visits the list", async () => {
    const h = await harness();
    try {
      await act(async () => h.navigation.visitLibraryTab("reader:a"));
      await act(async () => h.navigation.reconcileLibrary(["reader:a"], "reader:a"));
      await act(async () => h.navigation.navigate("notes"));
      await act(async () => h.navigation.navigate("library"));
      expect(h.navigation.location.tabId).toBe("reader:a");
      await act(async () => h.navigation.visitLibraryTab("library"));
      await act(async () => h.navigation.reconcileLibrary(["reader:a"], "library"));
      await act(async () => h.navigation.navigate("writing"));
      await act(async () => h.navigation.navigate("library"));
      expect(h.navigation.location.tabId).toBe("library");
    } finally { await act(async () => h.root.unmount()); }
  });
});

it("visits concrete note pages and restores their identity without new visits during traversal", async () => {
  let navigation!: ReturnType<typeof useWorkspaceNavigation>;
  function Harness() { navigation = useWorkspaceNavigation(); return null; }
  const root = createRoot(document.createElement("div"));
  try {
    await act(async () => root.render(<Harness />));
    await act(async () => navigation.navigate("notes"));
    await act(async () => navigation.visitNotePage("a"));
    await act(async () => navigation.visitNotePage("b"));
    await act(async () => navigation.visitNotePage("b"));
    await act(async () => navigation.back());
    expect(navigation.location).toEqual({ view: "notes", notePageId: "a" });
    await act(async () => navigation.forward());
    expect(navigation.location.notePageId).toBe("b");
    await act(async () => navigation.back());
    await act(async () => navigation.visitNotePage("c"));
    expect(navigation.next).toBeUndefined();
    await act(async () => navigation.navigate("settings"));
    await act(async () => navigation.navigate("notes"));
    expect(navigation.location.notePageId).toBe("c");
  } finally { await act(async () => root.unmount()); }
});

it("opens a supplied note destination through the same workspace navigation", async () => {
  let navigation!: ReturnType<typeof useWorkspaceNavigation>;
  function Harness() { navigation = useWorkspaceNavigation({ view: "notes", notePageId: "fixture" }); return null; }
  const root = createRoot(document.createElement("div"));
  try {
    await act(async () => root.render(<Harness />));
    expect(navigation.location).toEqual({ view: "notes", notePageId: "fixture" });
    expect(navigation.previous).toBeUndefined();
    await act(async () => navigation.navigate("library"));
    await act(async () => navigation.back());
    expect(navigation.location).toEqual({ view: "notes", notePageId: "fixture" });
  } finally { await act(async () => root.unmount()); }
});

it("keeps document-only note visits separate from management and restores presentation through history", async () => {
  let navigation!: ReturnType<typeof useWorkspaceNavigation>;
  function Harness() { navigation = useWorkspaceNavigation(); return null; }
  const root = createRoot(document.createElement("div"));
  try {
    await act(async () => root.render(<Harness />));
    await act(async () => navigation.navigate("notes", "page", "document"));
    expect(navigation.location).toEqual({view: "notes", notePageId: "page", presentation: "document", workspaceView: "library"});
    await act(async () => navigation.visitNotePage("linked"));
    expect(navigation.location.presentation).toBe("document");
    await act(async () => navigation.reconcileNotes(["page", "linked"], "linked"));
    await act(async () => navigation.navigate("notes"));
    expect(navigation.location).toEqual({view: "notes", notePageId: "linked"});
    await act(async () => navigation.back());
    expect(navigation.location).toEqual({view: "notes", notePageId: "linked", presentation: "document", workspaceView: "library"});
    await act(async () => navigation.forward());
    expect(navigation.location.presentation).toBeUndefined();
    await act(async () => navigation.visitNotesHome());
    expect(navigation.location).toEqual({view: "notes"});
  } finally { await act(async () => root.unmount()); }
});

it("clears document presentation when closing into a management sibling or the notes home", async () => {
  let navigation!: ReturnType<typeof useWorkspaceNavigation>;
  function Harness() { navigation = useWorkspaceNavigation(); return null; }
  const root = createRoot(document.createElement("div"));
  try {
    await act(async () => root.render(<Harness />));
    await act(async () => navigation.navigate("notes", "a", "document"));
    await act(async () => navigation.reconcileNotes(["b"], "b", "workspace"));
    expect(navigation.location).toEqual({view: "notes", notePageId: "b"});
    await act(async () => navigation.navigate("notes", "b", "document"));
    await act(async () => navigation.reconcileNotes([], undefined, "workspace"));
    expect(navigation.location).toEqual({view: "notes"});
  } finally { await act(async () => root.unmount()); }
});

it.each(["library", "writing", "notes"] as const)("keeps %s selected when either note adapter opens a document", async workspace => {
  let navigation!: ReturnType<typeof useWorkspaceNavigation>;
  function Harness() { navigation = useWorkspaceNavigation({view: workspace}); return null; }
  const root = createRoot(document.createElement("div"));
  try {
    await act(async () => root.render(<Harness />));
    await act(async () => navigation.navigate("notes", "reading-page", "document"));
    expect(navigation.view).toBe("notes"); expect(navigation.workspaceView).toBe(workspace);
    await act(async () => navigation.navigate("writing", undefined, "document"));
    expect(navigation.view).toBe("writing"); expect(navigation.workspaceView).toBe(workspace);
    await act(async () => navigation.back());
    expect(navigation.workspaceView).toBe(workspace);
    await act(async () => navigation.forward());
    expect(navigation.workspaceView).toBe(workspace);
    await act(async () => navigation.navigate("notes"));
    expect(navigation.workspaceView).toBe("notes"); expect(navigation.location.presentation).toBeUndefined();
  } finally { await act(async () => root.unmount()); }
});

it("returns to the research context when the last expanded page closes", async () => {
  let navigation!: ReturnType<typeof useWorkspaceNavigation>;
  function Harness() { navigation = useWorkspaceNavigation({view: "library", tabId: "reader:paper"}); return null; }
  const root = createRoot(document.createElement("div"));
  try {
    await act(async () => root.render(<Harness />));
    await act(async () => navigation.navigate("notes", "page", "document"));
    await act(async () => navigation.reconcileNotes([], undefined, "workspace"));
    expect(navigation.location).toEqual({view: "library", tabId: "reader:paper"});
    expect(navigation.workspaceView).toBe("library");
  } finally { await act(async () => root.unmount()); }
});

it("removes a deleted cascade from back and forward history while another workspace remains active", async () => {
  let navigation!: ReturnType<typeof useWorkspaceNavigation>;
  function Harness() {navigation = useWorkspaceNavigation(); return null;}
  const root = createRoot(document.createElement("div"));
  try {
    await act(async () => root.render(<Harness/>));
    await act(async () => navigation.visitNotePage("surviving"));
    await act(async () => navigation.visitNotePage("parent"));
    await act(async () => navigation.visitNotePage("child"));
    await act(async () => navigation.navigate("settings"));
    await act(async () => navigation.reconcileNotes(["surviving"], "surviving"));
    expect(navigation.location).toEqual({view: "settings"});
    await act(async () => navigation.back());
    expect(navigation.location).toEqual({view: "notes", notePageId: "surviving"});
    await act(async () => navigation.forward());
    expect(navigation.location).toEqual({view: "settings"});
    await act(async () => navigation.navigate("notes"));
    expect(navigation.location).toEqual({view: "notes", notePageId: "surviving"});
  } finally {await act(async () => root.unmount());}
});
