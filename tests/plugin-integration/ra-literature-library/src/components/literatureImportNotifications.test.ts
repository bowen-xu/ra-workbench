import type { ImportJob } from "@research-assistant/core/literature";
import { beforeEach, describe, expect, it, vi } from "vitest";

const notificationMocks = vi.hoisted(() => ({
  error: vi.fn(() => "error"),
  info: vi.fn(() => "info"),
  loading: vi.fn(() => "loading"),
  success: vi.fn(() => "success"),
  warning: vi.fn(() => "warning"),
}));

vi.mock("../../../../../apps/web/src/core/services/notifications", () => ({ notify: notificationMocks }));

import { showImportJobNotification } from "../../../../../../plugins/ra-literature-library/src/components/literatureImportNotifications";

function job(state: string): ImportJob {
  return {
    id: "job-1",
    state,
    sourceKind: "pdf",
    totalCount: 2,
    completedCount: state === "running" ? 1 : 2,
    cancellationRequested: false,
    results: state === "completed" ? [] : [{
      inputLabel: "broken.pdf",
      rawInput: "/hidden/broken.pdf",
      state: "failed",
      errorMessage: "不是有效 PDF",
    }],
    createdAt: "2026-08-06T00:00:00.000Z",
    updatedAt: "2026-08-06T00:00:00.000Z",
  };
}

describe("literature import notifications", () => {
  beforeEach(() => vi.clearAllMocks());

  it("shows a finite confirmation message for a DOI candidate instead of a running spinner", () => {
    showImportJobNotification({ ...job("queued"), sourceKind: "doi", totalCount: 1, completedCount: 0 },
      { onCancel: vi.fn(), onRetry: vi.fn() });
    expect(notificationMocks.loading).not.toHaveBeenCalled();
    expect(notificationMocks.info).toHaveBeenCalledWith("DOI 元数据待确认", expect.objectContaining({ id: "literature-import:job-1" }));
  });

  it("updates one global Sonner notification while an import is running", () => {
    const onCancel = vi.fn();
    showImportJobNotification(job("running"), { onCancel, onRetry: vi.fn() });
    expect(notificationMocks.loading).toHaveBeenCalledWith("正在导入 · 1/2", {
      id: "literature-import:job-1",
      className: "import-toast",
      description: "PDF",
      action: { label: "取消", onClick: onCancel },
    });
  });

  it("replaces progress with a top-level success message", () => {
    showImportJobNotification(job("completed"), { onCancel: vi.fn(), onRetry: vi.fn() });
    expect(notificationMocks.success).toHaveBeenCalledWith("已导入 2 篇文献", {
      id: "literature-import:job-1",
      className: "import-toast",
      description: "PDF",
    });
  });

  it("keeps failed details and retry in the Sonner notification", () => {
    const onRetry = vi.fn();
    showImportJobNotification(job("completed_with_errors"), { onCancel: vi.fn(), onRetry });
    expect(notificationMocks.warning).toHaveBeenCalledWith("导入结束 · 2/2", {
      id: "literature-import:job-1",
      className: "import-toast",
      description: "PDF · 2/2；broken.pdf：不是有效 PDF",
      duration: Infinity,
      action: { label: "重试 1 项", onClick: onRetry },
    });
  });
});
