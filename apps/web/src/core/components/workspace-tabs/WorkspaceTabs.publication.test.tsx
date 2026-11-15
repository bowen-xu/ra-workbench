/** @vitest-environment happy-dom */
import { act, memo, Profiler } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { WorkspaceTabsProvider, WorkspaceTabBar, usePublishWorkspaceTabs } from "./WorkspaceTabs";

it("does not commit an unchanged tab bar but dispatches the latest publisher callbacks", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host), committed = vi.fn(), selected = vi.fn();
  const Bar = memo(() => <Profiler id="tab-bar" onRender={committed}><WorkspaceTabBar owner="notes" textActive={false}
    navigate={(_owner, _destination, ready) => {ready(); return true;}}/></Profiler>);
  function Publisher({version, title}: {version: number; title: string}) {
    usePublishWorkspaceTabs("notes", [{id: "note", owner: "notes", active: true, title, select: () => selected(version)}]);
    return null;
  }
  const render = (version: number, title = "Retained note") => act(async () => root.render(<WorkspaceTabsProvider>
    <Publisher version={version} title={title}/><Bar/>
  </WorkspaceTabsProvider>));
  try {
    await render(0); committed.mockClear();
    for (let version = 1; version <= 20; version++) await render(version);
    if (process.env.WORKSPACE_PERF_REPORT) process.stdout.write(JSON.stringify({scenario: "20 unchanged publications", tabBarCommits: committed.mock.calls.length}) + "\n");
    expect(committed).not.toHaveBeenCalled();
    await act(async () => host.querySelector<HTMLButtonElement>('[role="tab"]')!.click());
    expect(selected).toHaveBeenCalledExactlyOnceWith(20);
    await render(21, "Renamed note");
    expect(host.textContent).toContain("Renamed note"); expect(committed).toHaveBeenCalledOnce();
  } finally { await act(async () => root.unmount()); host.remove(); vi.unstubAllGlobals(); }
});
