import { expect, it } from "vitest";
import { doclingErrorMessage } from "./doclingError";

it("retains structured Docling errors and excludes arbitrary diagnostic fields", () => {
  expect(doclingErrorMessage({ code: "start_failed", message: "Docling 服务启动超时", token: "hidden" }))
    .toBe("[start_failed] Docling 服务启动超时");
  expect(doclingErrorMessage(new Error("[request_failed] 连接失败"))).toBe("[request_failed] 连接失败");
  expect(doclingErrorMessage(Object.assign(new Error("连接失败"), { code: "request_failed" }))).toBe("[request_failed] 连接失败");
  expect(doclingErrorMessage("原有错误")).toBe("原有错误");
  expect(doclingErrorMessage({ code: "unsafe\ncode", message: "失败" })).toBe("失败");
  for (const reason of [null, undefined, {}, { message: {} }]) {
    expect(doclingErrorMessage(reason)).toBe("Docling 操作失败，请检查本地服务状态后重试");
  }
});
