// @vitest-environment happy-dom
import {afterEach, expect, it, vi} from "vitest";
import type {RuntimeLiteraturePort} from "@research-assistant/workbench-sdk/literature-runtime";
import {createRuntimeLiteraturePort, selectAppRuntime} from "../../../../../../integration/web/src/app/runtime/literature/runtimeLiteratureWorkspace.testSupport.tsx";
import {configureLiteratureServices} from "../../../../../../plugins/ra-literature-library/src/services/configureLiteratureServices";
import {getResearchExportGateway} from "../../../../../../plugins/ra-literature-library/src/services/researchExportGateway";

const key = "d".repeat(64);
const capabilities = [{id: "literature.research-export", version: 1}];
const report = {folderName: "synthetic-export", fileCount: 2, objectCount: 3, sourceCount: 1};
const select = (mode: "desktop" | "browser" = "desktop"): void => selectAppRuntime({mode, storage: localStorage, gateways: {}, dispose() {}});
afterEach(() => vi.restoreAllMocks());

it("uses the selected package, original report decoding and fixed cancellation", async () => {
  select();
  const invoke = vi.fn(async (command: string) => command === "export_runtime_research_knowledge" ? report : true);
  const release = configureLiteratureServices(createRuntimeLiteraturePort(key, capabilities, () => undefined, invoke));
  try {
    const gateway = getResearchExportGateway();
    await expect(gateway.exportAll("/synthetic/target")).resolves.toEqual(report);
    await expect(gateway.cancel()).resolves.toBe(true);
    expect(invoke.mock.calls).toEqual([
      ["export_runtime_research_knowledge", {packageKey: key, request: {targetDirectory: "/synthetic/target"}}],
      ["cancel_runtime_research_export", {packageKey: key}],
    ]);
    release();
    await expect(gateway.exportAll("/synthetic/target")).rejects.toMatchObject({code: "export_failed"});
    expect(invoke).toHaveBeenCalledTimes(2);
  } finally {release();}
});

it("keeps the original browser refusal even when a runtime port was configured", async () => {
  select("browser");
  const invoke = vi.fn();
  const release = configureLiteratureServices(createRuntimeLiteraturePort(key, capabilities, () => undefined, invoke));
  try {
    await expect(getResearchExportGateway().exportAll("/synthetic/target")).rejects.toMatchObject({code: "unavailable"});
    await expect(getResearchExportGateway().cancel()).resolves.toBe(false);
    expect(invoke).not.toHaveBeenCalled();
  } finally {release();}
});

it("rejects missing methods and capability version mismatch without invoking native", async () => {
  select();
  const invoke = vi.fn();
  const port = createRuntimeLiteraturePort(key, [{id: "literature.research-export", version: 2}], () => undefined, invoke);
  await expect(port.researchExport!.exportAll("/synthetic")).rejects.toMatchObject({code: "literature_gateway_unavailable"});
  const withoutExport = {...port};
  delete withoutExport.researchExport;
  const release = configureLiteratureServices(withoutExport as RuntimeLiteraturePort);
  try {await expect(getResearchExportGateway().exportAll("/synthetic")).rejects.toMatchObject({code: "export_failed"});}
  finally {release();}
  expect(invoke).not.toHaveBeenCalled();
});

it.each([{...report, fileCount: -1}, {...report, unexpected: true}])("rejects malformed original reports", async value => {
  select();
  const release = configureLiteratureServices(createRuntimeLiteraturePort(key, capabilities, () => undefined, async () => value));
  try {await expect(getResearchExportGateway().exportAll("/synthetic")).rejects.toThrow("不兼容的导出报告");}
  finally {release();}
});

it("retains native identity and safe cause through the original export error contract", async () => {
  select();
  const failure = {code: "plugin_host_unavailable", message: "Host ended", requestId: "request", instanceId: "instance", privateBody: "private"};
  const invoke = vi.fn(async () => {throw failure;});
  const release = configureLiteratureServices(createRuntimeLiteraturePort(key, capabilities, () => undefined, invoke));
  try {
    const error = await getResearchExportGateway().exportAll("/synthetic").catch(reason => reason);
    expect(error).toMatchObject({code: "export_failed", cause: {code: "plugin_host_unavailable", requestId: "request", instanceId: "instance", cause: failure}});
    expect(error.message).toContain("plugin_host_unavailable");
    expect(error.message).toContain("request");
    expect(error.message).not.toContain("private");
    expect(invoke).toHaveBeenCalledOnce();
  } finally {release();}
});
