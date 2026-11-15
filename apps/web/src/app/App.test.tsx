// @vitest-environment happy-dom

import { act, useEffect, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { App } from "./App";
import * as updateService from "../core/services/updateService";
import { selectAppStorage } from "../core/runtime/appStorage";
import type { ReadingNoteSource } from "@research-assistant/core/notes";
import { contextDirectoryGateway } from "../core/features/agent/context/contextDirectoryGateway";
const noteNavigation = vi.hoisted(() => ({ get: vi.fn(), recommend: vi.fn(), host: vi.fn() }));
const writingRequest = vi.hoisted(() => ({ current: null as null | {id?: string; key?: number; opened: () => void; failed?: (id: string, key: number | undefined, message?: string) => void} }));
vi.mock("../core/services/agentContextGateway", async importOriginal => {
  const actual = await importOriginal<typeof import("../core/services/agentContextGateway")>();
  return { ...actual, agentContextGateway: { ...actual.agentContextGateway, catalog: vi.fn(async () => []), recommend: noteNavigation.recommend } };
});
vi.mock("../core/features/notes/gateways", () => ({ getNotesGateway: () => ({ getReadingNote: noteNavigation.get }) }));

vi.mock("@tauri-apps/api/core", () => ({
  isTauri: () => false,
}));

vi.mock("./desktopLifecycle", () => ({
  installDesktopLifecycle: async () => () => undefined,
}));

vi.mock("../core/public/noteDocuments", async importOriginal => ({
  ...await importOriginal<typeof import("../core/public/noteDocuments")>(),
  CoreNoteWorkspace: () => null,
  CoreNoteWorkspaceHost: ({children, initialPageId}: {readonly children: import("react").ReactNode; readonly initialPageId?: string}) => {
    noteNavigation.host(initialPageId);
    return children;
  },
}));

vi.mock("@research-assistant/literature-library-plugin/workspace", () => ({
  LiteratureWorkspace: ({ openSource, onSourceOpened }: {
    openSource?: ReadingNoteSource;
    onSourceOpened: (opened: boolean) => void;
  }) => <section data-library-workspace>文献库工作区{openSource !== undefined && <>
    <span>已定位 {openSource.anchorId}</span>
    <button onClick={() => onSourceOpened(true)} type="button">PDF 已打开</button>
  </>}</section>,
}));

vi.mock("./notes/ReadingNoteFloatingPanel", async () => {
  const { useAgentContextCandidate, useAgentContextSuggestions } = await import("../core/features/agent/context/AgentContextProvider");
  return { ReadingNoteFloatingPanel: () => {
    const [opened, setOpened] = useState(false);
    useAgentContextCandidate(opened ? {resource:{kind:"note",resourceId:"new-note"},label:"新文献笔记"} : null);
    const resources = useAgentContextSuggestions();
    return <><button onClick={() => setOpened(true)}>打开新文献笔记夹具</button><output data-opened-context>{resources.map(item => item.label).join(",")}</output></>;
  } };
});

vi.mock("@research-assistant/note-manager-plugin/workspace", () => ({
  NotesWorkspace: ({ initialPageId }: { initialPageId?: string }) => <section data-note-workspace data-note-page={initialPageId}>笔记工作区</section>,
}));

vi.mock("@research-assistant/writing-projects-plugin/workspace", () => ({
  WritingWorkspace: ({ onEditorVisibilityChange, onOperationBusyChange, openManuscriptId, openManuscriptRequestKey, onManuscriptOpened, onManuscriptOpenFailed }: { onEditorVisibilityChange: (visible: boolean) => void; onOperationBusyChange: (busy:boolean) => void; openManuscriptId?: string; openManuscriptRequestKey?: number; onManuscriptOpened: () => void; onManuscriptOpenFailed?: (id: string, key: number | undefined, message?: string) => void }) => {
    writingRequest.current = { ...(openManuscriptId ? {id: openManuscriptId} : {}), ...(openManuscriptRequestKey === undefined ? {} : {key: openManuscriptRequestKey}), opened: onManuscriptOpened, ...(onManuscriptOpenFailed ? {failed: onManuscriptOpenFailed} : {}) };
    const [isEditorVisible, setIsEditorVisible] = useState(false);
    useEffect(() => {
      onEditorVisibilityChange(isEditorVisible);
      return () => onEditorVisibilityChange(false);
    }, [isEditorVisible, onEditorVisibilityChange]);
    return <section data-writing-workspace>论文写作工作区
      <button onClick={() => setIsEditorVisible(true)}>打开稿件</button>
      <button onClick={() => setIsEditorVisible(false)}>返回稿件列表</button>
      <button onClick={() => onOperationBusyChange(true)}>开始接受</button>
      <button onClick={() => onOperationBusyChange(false)}>结束接受</button>
    </section>;
  },
}));

vi.mock("../core/features/settings/LocalStorageSettings", () => ({
  LocalStorageSettings: () => <section>本地存储设置</section>,
}));

vi.mock("../core/features/settings/AiProviderSettings", () => ({
  AiProviderSettings: () => <section>AI 设置</section>,
}));

vi.mock("../core/features/settings/DoclingSettings", () => ({
  DoclingSettings: () => <section>Docling 设置</section>,
}));

async function flush(): Promise<void> {
  await act(async () => { await vi.dynamicImportSettled(); });
}

describe("App shell", () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    writingRequest.current = null;
    noteNavigation.get.mockReset();
    noteNavigation.host.mockReset();
    noteNavigation.recommend.mockReset();
    noteNavigation.recommend.mockImplementation(async candidate => candidate);
    const values = new Map<string,string>();
    selectAppStorage({get length(){return values.size;},key:index=>[...values.keys()][index]??null,getItem:key=>values.get(key)??null,setItem:(key,value)=>{values.set(key,value);},removeItem:key=>{values.delete(key);},clear:()=>values.clear()});
    (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    vi.stubGlobal("__APP_VERSION__", "test");
    container = document.createElement("div");
    document.body.append(container);
    root = createRoot(container);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
    vi.unstubAllGlobals();
  });
  it("restores the last workspace and global navigation collapse after a fresh mount",async()=>{
    await act(async()=>root.render(<App/>));await flush();
    await act(async()=>container.querySelector<HTMLButtonElement>('button[aria-label="笔记"]')!.click());await flush();
    await act(async()=>container.querySelector<HTMLButtonElement>('button[aria-controls="app-sidebar"]')!.click());
    await act(async()=>root.unmount());root=createRoot(container);
    await act(async()=>root.render(<App/>));await flush();
    expect(container.querySelector('.nav-item.active')?.getAttribute("aria-label")).toBe("笔记");
    expect(container.querySelector('[aria-controls="app-sidebar"]')?.getAttribute("aria-expanded")).toBe("false");
    expect(container.querySelector('.registered-workspace-host:has([data-note-workspace])')?.hasAttribute("hidden")).toBe(false);
  });

  it("routes canonical reading pages without shadowing a colliding ordinary page", async () => {
    const opened: string[] = [];
    const listener = (event: Event) => opened.push((event as CustomEvent<string>).detail);
    window.addEventListener("research-assistant:open-reading-note", listener);
    try {
      noteNavigation.get.mockResolvedValue({ id: "legacy", pageId: "canonical" });
      await act(async () => root.render(<App />)); await flush();
      await act(async () => window.dispatchEvent(new CustomEvent("research-assistant:open-note-page", { detail: "canonical" })));
      expect(opened).toEqual(["legacy"]); expect(container.textContent).toContain("文献库工作区");
      await act(async () => window.dispatchEvent(new CustomEvent("research-assistant:open-note-page", { detail: "legacy" })));
      await flush(); expect(opened).toEqual(["legacy"]);
      expect(noteNavigation.host).toHaveBeenLastCalledWith("legacy");
    } finally { window.removeEventListener("research-assistant:open-reading-note", listener); }
  });

  it("registers a newly opened floating note in the same global composer context", async () => {
    await act(async () => root.render(<App />)); await flush();
    const open = [...container.querySelectorAll("button")].find(button => button.textContent === "打开新文献笔记夹具")!;
    await act(async () => open.click()); await flush();
    expect(noteNavigation.recommend).toHaveBeenCalledExactlyOnceWith({resource:{kind:"note",resourceId:"new-note"},label:"新文献笔记"}, expect.any(AbortSignal));
    expect(container.querySelector("[data-opened-context]")?.textContent).toContain("新文献笔记");
  });

  it("starts in the literature library without project or evidence workspaces", async () => {
    await act(async () => root.render(<App />));
    await flush();

    const navText = container.querySelector(".nav-list")?.textContent ?? "";
    expect(navText).toContain("文献");
    expect(navText).toContain("笔记");
    expect(navText).toContain("写作");
    expect(navText).toContain("设置");
    expect(navText).not.toContain("研究项目");
    expect(navText).not.toContain("证据工作台");
    expect(container.textContent).toContain("文献库工作区");
    expect(container.querySelectorAll('[aria-label="打开对话"]')).toHaveLength(1);
  });

  it("opens historical chats in the shared canvas without requesting another writing project", async () => {
    const read = vi.spyOn(contextDirectoryGateway, "read");
    try {
      await act(async () => root.render(<App initialLocation={{view: "writing"}}/>)); await flush();
      for (const id of ["first-conversation", "second-conversation"]) {
        const resolve = vi.fn(), reject = vi.fn();
        await act(async () => window.dispatchEvent(new CustomEvent("research-assistant:open-agent-conversation", {cancelable: true, detail: {id, resolve, reject}})));
        await flush();
        expect(reject).not.toHaveBeenCalled(); expect(resolve).toHaveBeenCalledOnce();
        expect(writingRequest.current?.id).toBeUndefined();
      }
      expect(read).not.toHaveBeenCalled();
      expect(container.querySelector(".registered-workspace-host:has([data-writing-workspace])")?.hasAttribute("hidden")).toBe(false);
    } finally {read.mockRestore();}
  });

  it("keeps settings free of a unified AI review inbox", async () => {
    await act(async () => root.render(<App />));
    await flush();

    const settings = [...container.querySelectorAll("button")]
      .find((button) => button.textContent?.includes("设置") === true);
    await act(async () => settings?.click());
    await flush();

    expect(container.querySelector('.settings-page:not([hidden])')?.textContent).toContain("常规");
    expect(container.textContent).not.toContain("检查并安装更新");
    expect(container.querySelector('[aria-label="检查更新"]')).not.toBeNull();
    const storage = [...container.querySelectorAll<HTMLButtonElement>('.settings-navigation button')].find(button => button.textContent === "存储");
    await act(async () => storage?.click()); await flush();
    expect(container.textContent).toContain("本地存储设置");
    expect(container.querySelector(".ai-review-inbox")).toBeNull();
    expect(container.textContent).not.toContain("AI 建议审查");
    expect(container.textContent).not.toContain("AI 候选管理");
  });

  it("checks updates only from the shell action, not when entering settings", async () => {
    const check = vi.fn(async () => ({ ...updateService.INITIAL_UPDATE_STATUS, revision: 1, phase: "upToDate" as const, message: "当前已是最新版本。" }));
    const gateway = vi.spyOn(updateService, "createUpdateGateway").mockReturnValue({
      status: async () => updateService.INITIAL_UPDATE_STATUS, subscribe: async () => () => undefined,
      check, download: vi.fn(), pause: vi.fn(), install: vi.fn(),
    });
    try {
      await act(async () => root.render(<App initialLocation={{ view: "settings" }} />)); await flush();
      expect(check).not.toHaveBeenCalled();
      const button = container.querySelector<HTMLButtonElement>('[aria-label="检查更新"]');
      await act(async () => button?.click()); expect(check).toHaveBeenCalledOnce();
      await vi.waitFor(async () => {
        await flush();
        expect(document.body.textContent).toContain("当前已是最新版本。");
      });
    } finally { gateway.mockRestore(); }
  });

  it("routes local source inspection through the literature reader without a review inbox", async () => {
    await act(async () => root.render(<App />));
    await flush();
    const click = async (label: string): Promise<void> => {
      const button = [...container.querySelectorAll("button")]
        .find((item) => (item.getAttribute("aria-label") ?? item.textContent) === label);
      expect(button, label).toBeDefined();
      await act(async () => button?.click());
      await flush();
    };

    await click("设置");
    const resolve = vi.fn();
    const reject = vi.fn();
    await act(async () => window.dispatchEvent(new CustomEvent("research-assistant:inspect-reading-source", {
      cancelable: true,
      detail: { source: {anchorId: "anchor-1", paperId: "paper-1", paperTitle: "Paper", attachmentId: "attachment-1", contentSha256: "a".repeat(64), pageIndex: 1, rects: [], quoteExact: "Quote", quotePrefix: "", quoteSuffix: "", state: "exact"}, resolve, reject },
    })));
    await flush();
    expect(container.textContent).toContain("文献库工作区");
    expect(container.textContent).toContain("已定位 anchor-1");
    expect(resolve).not.toHaveBeenCalled();
    await click("PDF 已打开");
    expect(resolve).toHaveBeenCalledOnce();
    expect(reject).not.toHaveBeenCalled();
  });

  it("keeps AI API settings in settings and removes the duplicate shell utility", async () => {
    await act(async () => root.render(<App initialLocation={{view: "settings"}} />));
    await flush();

    expect(container.querySelector('#app-sidebar button[aria-label="配置 AI API"]')).toBeNull();
    const button = [...container.querySelectorAll<HTMLButtonElement>('.setting-row button')].find(item => item.textContent === "管理");
    expect(button).toBeDefined();
    await act(async () => button?.click());
    await flush();

    expect(container.textContent).toContain("AI 设置");
    expect(container.textContent).not.toContain("正在读取 AI API 设置…");
  });

  it("keeps the writing host mounted during acceptance for navigation and external events", async () => {
    await act(async () => root.render(<App />)); await flush();
    async function click(label: string) {
      const button = [...container.querySelectorAll("button")].find(item => (item.getAttribute("aria-label") ?? item.textContent) === label);
      expect(button).toBeDefined();
      await act(async () => button?.click()); await flush();
    }
    await click("写作"); await click("开始接受");
    expect(container.querySelector<HTMLButtonElement>('button[aria-label="文献"]')?.disabled).toBe(true);
    await click("文献");
    for (const event of [new Event("research-assistant:open-notes"),
      new CustomEvent("research-assistant:open-reading-source", {detail:{}}),
      new CustomEvent("research-assistant:open-research-target", {detail:{kind:"paper",id:"paper"}})]) {
      await act(async () => {window.dispatchEvent(event);}); await flush();
      expect(container.textContent).toContain("论文写作工作区");
      expect(document.body.textContent).toContain("稿件提案操作尚未结束");
    }
    await click("结束接受"); await click("笔记");
    expect(container.textContent).toContain("笔记工作区");
    expect(container.querySelector<HTMLElement>(".main-area > div[hidden]")).not.toBeNull();
  });

  it("preserves the global sidebar choice across navigation and isolates the temporary editor override", async () => {
    await act(async () => root.render(<App />));
    await flush();
    const expanded = (): string | null | undefined => container.querySelector('[aria-controls="app-sidebar"]')?.getAttribute("aria-expanded");
    const click = async (name: string): Promise<void> => {
      const button = [...container.querySelectorAll("button")].find((item) => (item.getAttribute("aria-label") ?? item.textContent) === name);
      expect(button, name).toBeDefined();
      await act(async () => button?.click());
      await flush();
    };

    expect(expanded()).toBe("true");
    await click("收起主导航");
    expect(expanded()).toBe("false");
    expect(container.querySelector('button[aria-label="文献"] svg')).not.toBeNull();
    expect(container.querySelector('button[aria-label="笔记"] svg')).not.toBeNull();
    expect(container.querySelector('button[aria-label="写作"] svg')).not.toBeNull();
    expect(container.querySelector('button[aria-label="写作"]')?.textContent).not.toContain("✎");
    for (const page of ["笔记", "设置", "写作", "文献"]) {
      await click(page);
      expect(expanded()).toBe("false");
    }
    await click("写作");
    await click("打开稿件");
    await click("展开主导航");
    expect(expanded()).toBe("true");
    await click("返回稿件列表");
    expect(expanded()).toBe("false");
    await click("展开主导航");
    expect(expanded()).toBe("true");
    for (const page of ["文献", "笔记", "设置", "写作"]) {
      await click(page);
      expect(expanded()).toBe("true");
      expect(container.querySelectorAll('[aria-label="打开对话"]')).toHaveLength(1);
    }
    await click("打开稿件");
    expect(expanded()).toBe("true");
    await act(async () => root.render(<App />));
    expect(expanded()).toBe("true");
    await click("返回稿件列表");
    expect(expanded()).toBe("true");
    await click("打开稿件");
    expect(expanded()).toBe("true");
    await click("笔记");
    expect(expanded()).toBe("true");
    await click("写作");
    expect(expanded()).toBe("true");
    expect(container.querySelector('.app-window-toolbar [aria-controls="app-sidebar"]')).not.toBeNull();
  });

  it("uses window navigation history without wrapping or retaining abandoned forward entries", async () => {
    await act(async () => root.render(<App />)); await flush();
    const button = (name: string) => [...container.querySelectorAll<HTMLButtonElement>("button")].find(item => (item.getAttribute("aria-label") ?? item.textContent) === name)!;
    const click = async (name: string) => { await act(async () => button(name).click()); await flush(); };
    expect(button("后退").disabled).toBe(true); expect(button("前进").disabled).toBe(true);
    await click("笔记"); await click("写作");
    await click("后退"); expect(container.textContent).toContain("笔记工作区");
    expect(button("前进").disabled).toBe(false);
    await click("前进"); expect(container.textContent).toContain("论文写作工作区");
    await click("开始接受"); expect(button("后退").disabled).toBe(true);
    await click("结束接受"); await click("后退"); await click("设置");
    expect(button("前进").disabled).toBe(true);
    await click("后退"); await click("后退");
    expect(button("后退").disabled).toBe(true); expect(container.textContent).toContain("文献库工作区");
  });
});
