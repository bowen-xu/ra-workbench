// @vitest-environment happy-dom

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type {
  PaperReadingProfileSelection,
  ReadingProfile,
} from "@research-assistant/core/knowledge";

import type { ResearchKnowledgeGateway } from "../../../../../../../plugins/ra-literature-library/src/services/knowledgeGateway";
import { notify } from "@research-assistant/workbench-sdk/feedback";
import { ReadingProfileSelector } from "../../../../../../../plugins/ra-literature-library/src/features/ai/ReadingProfileSelector";

vi.mock("../../../../../../apps/web/src/core/services/notifications", () => ({
  notify: {
    error: vi.fn(),
    success: vi.fn(),
  },
}));

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean })
  .IS_REACT_ACT_ENVIRONMENT = true;

const profiles: readonly ReadingProfile[] = [
  {
    profileId: "generic",
    profileVersion: 1,
    profileKind: "generic",
    label: "通用阅读",
    description: "适合尚未确定论文类型时的基础阅读框架。",
    fieldKinds: ["research_question"],
    reviewStatus: "approved",
    createdAt: "2026-01-01T00:00:00.000Z",
  },
  {
    profileId: "technical",
    profileVersion: 1,
    profileKind: "technical_empirical",
    label: "技术 / 实证",
    description: "关注研究问题、方法或系统、关键结果。",
    fieldKinds: ["approach", "key_result"],
    reviewStatus: "approved",
    createdAt: "2026-01-01T00:00:00.000Z",
  },
  {
    profileId: "philosophical",
    profileVersion: 1,
    profileKind: "philosophical_conceptual",
    label: "哲学 / 概念",
    description: "关注核心问题、论证、异议与回应。",
    fieldKinds: ["argument", "objection", "response"],
    reviewStatus: "approved",
    createdAt: "2026-01-01T00:00:00.000Z",
  },
];

const savedGeneric: PaperReadingProfileSelection = {
  paperId: "paper-1",
  profiles: [{ profileId: "generic", profileVersion: 1 }],
  revision: 5,
  isPersisted: true,
};

function createGateway(
  saveResult: PaperReadingProfileSelection = savedGeneric,
): ResearchKnowledgeGateway {
  return {
    listReadingProfiles: vi.fn(async () => profiles),
    getPaperReadingProfileSelection: vi.fn(async () => savedGeneric),
    savePaperReadingProfileSelection: vi.fn(async () => saveResult),
  } as unknown as ResearchKnowledgeGateway;
}

describe("ReadingProfileSelector", () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    container = document.createElement("div");
    document.body.append(container);
    root = createRoot(container);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
  });

  async function renderSelector(
    gateway: ResearchKnowledgeGateway,
    onSelectionChanged = vi.fn(),
  ): Promise<void> {
    await act(async () => root.render(
      <ReadingProfileSelector
        busy={false}
        gateway={gateway}
        onSelectionChanged={onSelectionChanged}
        paperId="paper-1"
      />,
    ));
    await act(async () => undefined);
  }

  it("shows a compact saved summary and opens the options only on demand", async () => {
    await renderSelector(createGateway());

    expect(document.querySelector(".reading-profile-options")).toBeNull();
    expect(container.querySelector(".reading-profile-current")?.textContent).toBe("通用阅读");
    expect(container.querySelector(".reading-profile-summary-main")?.textContent).not.toContain("v5");
    expect(container.querySelector('[aria-label^="阅读框架：可信"]')).not.toBeNull();

    const trigger = container.querySelector<HTMLButtonElement>('[aria-label="展开阅读框架选择"]')!;
    expect(trigger.querySelector(".lucide-chevron-left")).not.toBeNull();
    await act(async () => trigger.click());

    expect(document.querySelector('[aria-label="配置阅读框架"]')).not.toBeNull();
    expect(document.querySelectorAll(".reading-profile-options > label:not(.reading-profile-unspecified)")).toHaveLength(3);
    expect(document.querySelector('.reading-profile-unspecified input[type="radio"]')).not.toBeNull();
    expect(document.querySelector(".reading-profile-unspecified .content-trust-indicator")).toBeNull();
    expect(document.querySelector(".reading-profile-options")?.textContent).not.toContain("v1");
    expect(trigger.getAttribute("aria-expanded")).toBe("true");
    expect(trigger.getAttribute("aria-label")).toBe("收起阅读框架选择");
    expect(trigger.querySelector(".lucide-chevron-down")).not.toBeNull();
  });

  it("keeps draft changes out of the active selection and discards them with Escape", async () => {
    const onSelectionChanged = vi.fn();
    await renderSelector(createGateway(), onSelectionChanged);
    onSelectionChanged.mockClear();

    const trigger = container.querySelector<HTMLButtonElement>('[aria-label="展开阅读框架选择"]')!;
    await act(async () => trigger.click());
    const checkboxes = document.querySelectorAll<HTMLInputElement>('.reading-profile-options input[type="checkbox"]');
    await act(async () => checkboxes[0]!.click());
    await act(async () => checkboxes[1]!.click());

    expect(onSelectionChanged).not.toHaveBeenCalled();
    await act(async () => document.activeElement?.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, key: "Escape" })));

    expect(document.querySelector('[aria-label="配置阅读框架"]')).toBeNull();
    expect(container.querySelector(".reading-profile-current")?.textContent).toBe("通用阅读");
    expect(document.activeElement).toBe(trigger);
  });

  it("saves an explicit unspecified selection and returns to the compact summary", async () => {
    const savedUnspecified: PaperReadingProfileSelection = {
      paperId: "paper-1",
      profiles: [],
      revision: 6,
      isPersisted: true,
    };
    const gateway = createGateway(savedUnspecified);
    await renderSelector(gateway);

    const trigger = container.querySelector<HTMLButtonElement>('[aria-label="展开阅读框架选择"]')!;
    await act(async () => trigger.click());
    const unspecified = document.querySelector<HTMLLabelElement>(".reading-profile-unspecified")!;
    await act(async () => unspecified.click());
    const save = [...document.querySelectorAll<HTMLButtonElement>(".reading-profile-popover footer button")]
      .find((button) => button.textContent === "保存")!;
    await act(async () => save.click());

    expect(gateway.savePaperReadingProfileSelection).toHaveBeenCalledWith({
      paperId: "paper-1",
      profiles: [],
      expectedRevision: 5,
      changeReason: "manual_profile_confirmation",
    });
    expect(document.querySelector('[aria-label="配置阅读框架"]')).toBeNull();
    expect(container.querySelector(".reading-profile-current")?.textContent).toBe("未指定");
    expect(container.querySelector('[aria-label^="阅读框架：无可审查内容"]')).not.toBeNull();
    expect(vi.mocked(notify.success)).toHaveBeenCalledWith("未指定阅读框架已保存");
    expect(vi.mocked(notify.success).mock.calls.flat().join(" ")).not.toContain("v6");
  });

  it("closes and discards its draft when the user clicks outside", async () => {
    await renderSelector(createGateway());

    const trigger = container.querySelector<HTMLButtonElement>('[aria-label="展开阅读框架选择"]')!;
    await act(async () => trigger.click());
    await act(async () => {
      document.body.dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));
      document.body.dispatchEvent(new MouseEvent("mouseup", { bubbles: true }));
      document.body.click();
    });

    expect(document.querySelector('[aria-label="配置阅读框架"]')).toBeNull();
    expect(document.activeElement).toBe(trigger);
  });
});
