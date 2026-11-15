/** @vitest-environment happy-dom */
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { AgentSkillControl } from "./AgentSkillControl";

const gateway = vi.hoisted(() => ({ list: vi.fn(), setEnabled: vi.fn() }));
const notifications = vi.hoisted(() => ({ error: vi.fn(), success: vi.fn() }));
vi.mock("../../services/agentGateway", () => ({ agentSkillCatalogGateway: gateway }));
vi.mock("../../services/notifications", () => ({ notify: notifications }));
(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const catalog = { schemaVersion: 1 as const, environmentId: "application" as const, skills: [{ sourceId: ".agents/skills/review/SKILL.md", name: "review", description: "Review", revision: "sha256", allowImplicitInvocation: true, enabled: true }], errors: [] };

afterEach(() => { document.body.replaceChildren(); vi.clearAllMocks(); });

it("loads the application catalog and toggles by exact revision", async () => {
  gateway.list.mockResolvedValue(catalog);
  gateway.setEnabled.mockResolvedValue({ ...catalog, skills: [{ ...catalog.skills[0], enabled: false }] });
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  await act(async () => root.render(createElement(AgentSkillControl)));
  await act(async () => document.querySelector("button")?.click());
  expect(document.body.textContent).toContain("review");
  await act(async () => [...document.querySelectorAll("button")].at(-1)?.click());
  expect(gateway.setEnabled).toHaveBeenCalledWith(".agents/skills/review/SKILL.md", "sha256", false);
  expect(notifications.success).toHaveBeenCalled();
  await act(async () => root.unmount());
});

it("publishes enabled skills for a workflow before its menu is opened", async () => {
  gateway.list.mockResolvedValue(catalog);
  const onSelectionChange = vi.fn();
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  await act(async () => root.render(createElement(AgentSkillControl, { onSelectionChange })));
  expect(gateway.list).toHaveBeenCalledTimes(1);
  expect(onSelectionChange).toHaveBeenCalledWith([".agents/skills/review/SKILL.md"]);
  await act(async () => root.render(createElement(AgentSkillControl, { onSelectionChange: () => undefined })));
  expect(gateway.list).toHaveBeenCalledTimes(1);
  await act(async () => root.unmount());
});

it("ignores an older catalog response after a newer load starts", async () => {
  let resolveFirst!: (value: typeof catalog) => void;
  let resolveSecond!: (value: typeof catalog) => void;
  gateway.list
    .mockImplementationOnce(() => new Promise((resolve) => { resolveFirst = resolve; }))
    .mockImplementationOnce(() => new Promise((resolve) => { resolveSecond = resolve; }));
  const onSelectionChange = vi.fn();
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  await act(async () => root.render(createElement(AgentSkillControl, { onSelectionChange })));
  await act(async () => document.querySelector("button")?.click());
  await act(async () => resolveSecond({ ...catalog, skills: [{ ...catalog.skills[0]!, enabled: false }] }));
  await act(async () => resolveFirst(catalog));
  expect(onSelectionChange).toHaveBeenCalledTimes(1);
  expect(onSelectionChange).toHaveBeenCalledWith([]);
  await act(async () => root.unmount());
});

it("drops a late skill update after the control is unmounted", async () => {
  gateway.list.mockResolvedValue(catalog);
  let resolveUpdate!: (value: typeof catalog) => void;
  gateway.setEnabled.mockImplementation(() => new Promise((resolve) => { resolveUpdate = resolve; }));
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  await act(async () => root.render(createElement(AgentSkillControl)));
  await act(async () => document.querySelector("button")?.click());
  await act(async () => document.querySelectorAll("[role=menuitem]")[0]?.dispatchEvent(new MouseEvent("click", { bubbles: true })));
  await act(async () => root.unmount());
  await act(async () => resolveUpdate({ ...catalog, skills: [{ ...catalog.skills[0]!, enabled: false }] }));
  expect(notifications.success).not.toHaveBeenCalled();
  expect(notifications.error).not.toHaveBeenCalled();
});
