import {withSourceNamespaces} from "./sourceDirectoryFixtures";
// @vitest-environment happy-dom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { AgentSourceLink } from "./AgentSourceLink";
import { AgentAnnotationCard } from "./AgentAnnotationCard";
import { panelItem } from "../../../../../../plugins/literature_library/src/features/ai/paperOverviewTestFixtures";

const ports = vi.hoisted(() => ({ directory: vi.fn(), developerMode: false }));
vi.mock("./sourceDirectory", () => ({ sourceDirectory: ports.directory }));
vi.mock("@research-assistant/literature-library-plugin/literatureGateway", () => ({ getLiteratureGateway: () => ({}) }));
vi.mock("../notes/gateways", () => ({ getNotesGateway: () => ({}) }));
vi.mock("@research-assistant/writing-projects-plugin/writingGateway", () => ({ getWritingGateway: () => ({}) }));
vi.mock("../settings/DeveloperMode", () => ({ useDeveloperMode: () => ({enabled: ports.developerMode}) }));
vi.mock("../../components/ControlHint", () => ({ControlHint: ({children, content}: {children: React.ReactNode; content: React.ReactNode}) => <>{children}<aside>{content}</aside></>}));
const cleanups: (() => void)[] = [];
afterEach(() => { cleanups.splice(0).forEach(cleanup => cleanup()); ports.developerMode = false; vi.clearAllMocks(); vi.unstubAllGlobals(); });

it.each([
  ["paper", "newspaper", "tex"], ["pdf", "file", "tex"], ["note", "sticky-note", "tex"],
  ["highlight", "highlighter", "tex"], ["manuscript", "folder-pen", "tex"], ["folder", "folder", "tex"],
  ["manuscriptFile", "file-code-corner", "tex"], ["manuscriptFile", "image", "image"],
])("uses the resolved %s type for its icon even with an identical label", async (kind, icon, fileKind) => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  ports.directory.mockReturnValue(withSourceNamespaces({ value: { kind: "root" }, child: async () => ({ value: {
    kind, item: panelItem(), attachment: { displayName: "paper.pdf" }, snapshot: { page: { title: "笔记名称" } },
    manuscript: { title: "项目名称" }, file: { kind: fileKind, relativePath: "main.tex" },
    note: { source: { paperTitle: "文献名称" } }, path: "sections",
  }, child: async () => undefined }) }));
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host);
  cleanups.push(() => { act(() => root.unmount()); host.remove(); });
  await act(async () => root.render(<AgentSourceLink href="source://notes/object" path="source://notes/object">同一名称</AgentSourceLink>));
  const link = host.querySelector("a")!;
  expect(link.textContent).toBe("同一名称");
  expect(link.classList.contains("agent-composer-reference")).toBe(true);
  expect(link.querySelector(`.lucide-${icon}`)?.getAttribute("aria-hidden")).toBe("true");
});

it("retains a generic icon when the source no longer resolves", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  ports.directory.mockReturnValue(withSourceNamespaces({ value: { kind: "root" }, child: async () => undefined }));
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host);
  cleanups.push(() => { act(() => root.unmount()); host.remove(); });
  await act(async () => root.render(<AgentSourceLink href="source://notes/missing" path="source://notes/missing">来源</AgentSourceLink>));
  expect(host.querySelector(".lucide-link-2")).not.toBeNull();
  expect(host.querySelector("a")?.getAttribute("href")).toBe("source://notes/missing");
  expect(host.querySelector("aside")?.textContent).toBe("来源不可用");
});

it("shows the exact source path in developer hints even for an unavailable source", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  ports.developerMode = true;
  ports.directory.mockReturnValue(withSourceNamespaces({value: {kind: "root"}, child: async () => undefined}));
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host);
  cleanups.push(() => { act(() => root.unmount()); host.remove(); });
  const path = "source://notes/missing/file:#line{1-1:1-8}";
  await act(async () => root.render(<AgentSourceLink href={path} path={path}>来源</AgentSourceLink>));
  expect(host.querySelector(".agent-source-hint-path")?.textContent).toBe(path);
});

it("uses the shared readable hint on the whole annotation, without exposing its URI in developer mode", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  ports.developerMode = true;
  ports.directory.mockReturnValue(withSourceNamespaces({value: {kind: "root"}, child: async () => ({value: {
    kind: "pdf", item: panelItem(), attachment: {displayName: "paper.pdf"},
  }, child: async () => undefined})}));
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host);
  cleanups.push(() => {act(() => root.unmount()); host.remove();});
  const path = "source://library/paper:#line{1:2-1:3-8}";
  await act(async () => root.render(<AgentAnnotationCard item={{type:"annotation",source:path,title:"原文引用",text:"原文",comment:""}} disabled={false}/>));
  expect(host.querySelector("a")).toBeNull();
  expect(host.querySelector("article")?.getAttribute("tabindex")).toBe("0");
  expect(host.querySelector("aside")?.textContent).toContain("paper.pdf · 第 1 页 · line 2-1:3-8");
  expect(host.querySelector("aside")?.textContent).not.toContain("source://");
});
