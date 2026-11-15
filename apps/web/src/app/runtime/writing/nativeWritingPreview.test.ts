import {beforeEach, expect, it, vi} from "vitest";
import {PluginPackageError, WORKBENCH_UI_RUNTIME} from "@research-assistant/workbench-sdk";
import type {invoke} from "../../../core/runtime/platformCore";
import {createNativeWritingPreview} from "./nativeWritingPreview";
import {tauriWritingGateway} from "./tauriWritingGateway";

const platform = vi.hoisted(() => ({invoke: vi.fn()}));
vi.mock("../../../core/runtime/platformCore", () => ({invoke: platform.invoke}));
beforeEach(() => platform.invoke.mockReset());

const packageKey = "a".repeat(64);
const sourcePath = "  /synthetic/project.zip  ";
const receipt = Object.freeze({fileCount: 2, byteSize: 128, rootCandidates: ["main.tex"]});

function manifest(overrides: Record<string, unknown> = {}): Record<string, unknown> {
  return {formatVersion: 2, id: "writing_projects", version: "1.0.0", apiVersion: 1,
    native: {protocolVersion: 1, os: "macos", arch: "aarch64", entry: "native/writing-host"},
    dependencies: {required: [], optional: []}, provides: [{id: "writing.import-preview", version: 1}],
    consumes: {required: [], optional: []}, ...overrides};
}

function catalog(nativeWritingPreview = true, metadata: Record<string, unknown> = manifest()) {
  return {nativeWritingPreview, packages: [{packageKey, manifest: metadata}]};
}

function setup(discovered: unknown = catalog()) {
  const invokeCommand = vi.fn<typeof invoke>().mockResolvedValueOnce(discovered);
  // Vitest erases a generic function's T to unknown; the fixture supplies each
  // command's actual typed receipt while the production invoke stays generic.
  return {invokeCommand, preview: createNativeWritingPreview(invokeCommand as typeof invoke)};
}

it("keeps the exact static command, source and receipt when Core disables the trial", async () => {
  // Disabled mode does not turn unrelated package metadata into a new static
  // product gate; discovery envelope types still have to be valid.
  const state = setup({nativeWritingPreview: false, packages: [{manifest: "not preflighted"}]});
  state.invokeCommand.mockResolvedValueOnce(receipt);
  await expect(state.preview(sourcePath)).resolves.toBe(receipt);
  expect(state.invokeCommand.mock.calls).toEqual([
    ["list_runtime_plugin_packages"], ["preview_manuscript_import", {sourcePath}],
  ]);
});

it("uses the real production gateway and only forwards the selected content key and source", async () => {
  platform.invoke.mockResolvedValueOnce(catalog()).mockResolvedValueOnce(receipt);
  await expect(tauriWritingGateway.previewImport(sourcePath)).resolves.toBe(receipt);
  expect(platform.invoke.mock.calls).toEqual([
    ["list_runtime_plugin_packages"], ["preview_runtime_manuscript_import", {packageKey, sourcePath}],
  ]);
});

it("reads Core's switch and catalog for each explicit user invocation without replay", async () => {
  const state = setup();
  state.invokeCommand.mockResolvedValueOnce(receipt).mockResolvedValueOnce(catalog(false)).mockResolvedValueOnce(receipt);
  await state.preview(sourcePath);
  await state.preview(sourcePath);
  expect(state.invokeCommand.mock.calls.map(([command]) => command)).toEqual([
    "list_runtime_plugin_packages", "preview_runtime_manuscript_import",
    "list_runtime_plugin_packages", "preview_manuscript_import",
  ]);
});

it.each([
  [null, "discovery"], [[], "discovery"], [{packages: []}, "nativeWritingPreview"],
  [{nativeWritingPreview: "true", packages: []}, "nativeWritingPreview"],
  [{nativeWritingPreview: 0, packages: []}, "nativeWritingPreview"],
  [{nativeWritingPreview: false}, "packages"], [{nativeWritingPreview: true, packages: {}}, "packages"],
] as const)("rejects a malformed Core configuration envelope without choosing a fallback", async (input, field) => {
  const state = setup(input);
  await expect(state.preview(sourcePath)).rejects.toMatchObject({code: "plugin_discovery_invalid", diagnostic: {field}});
  expect(state.invokeCommand).toHaveBeenCalledExactlyOnceWith("list_runtime_plugin_packages");
});

it("reports a missing enabled writing package instead of using the static implementation", async () => {
  const state = setup({nativeWritingPreview: true, packages: []});
  await expect(state.preview(sourcePath)).rejects.toMatchObject({code: "plugin_host_package_missing", pluginId: "writing_projects"});
  expect(state.invokeCommand).toHaveBeenCalledTimes(1);
});

it.each([
  {provides: [{id: "writing.projects", version: 1}]},
  {provides: [{id: "writing.import-preview", version: 2}]},
  {provides: [{id: "writing.import-preview", version: 1}, {id: "writing.projects", version: 1}]},
  {entry: "writing.js", styles: [], uiRuntime: WORKBENCH_UI_RUNTIME},
  {dependencies: {required: [], optional: [{id: "literature_library", apiVersion: 1}]}},
  // Resolves against the actual Core catalog before the narrower B profile is
  // checked. An empty replacement Core capability list would fail differently.
  {consumes: {required: [{id: "core.resources", version: 1, provider: "core"}], optional: []}},
])("rejects a writing package outside the frozen profile before a native call", async overrides => {
  const state = setup(catalog(true, manifest(overrides)));
  await expect(state.preview(sourcePath)).rejects.toMatchObject({code: "plugin_host_profile_unsupported", packageKey});
  expect(state.invokeCommand).toHaveBeenCalledTimes(1);
});

it("retains the shared decoder's error identity and field for package-chosen execution options", async () => {
  const state = setup(catalog(true, manifest({native: {protocolVersion: 1, os: "macos", arch: "aarch64",
    entry: "native/host", args: ["not permitted"]}})));
  const pending = state.preview(sourcePath);
  await expect(pending).rejects.toBeInstanceOf(PluginPackageError);
  await expect(pending).rejects.toMatchObject({code: "plugin_manifest_invalid", diagnostic: {
    pluginId: "writing_projects", field: "native.args",
  }});
  expect(state.invokeCommand).toHaveBeenCalledTimes(1);
});

it("retains the shared protocol compatibility diagnostic without invoking either preview command", async () => {
  const state = setup(catalog(true, manifest({native: {protocolVersion: 2, os: "macos", arch: "aarch64", entry: "native/host"}})));
  await expect(state.preview(sourcePath)).rejects.toMatchObject({code: "plugin_native_protocol_incompatible", diagnostic: {
    pluginId: "writing_projects", expectedVersion: 1, actualVersion: 2,
  }});
  expect(state.invokeCommand).toHaveBeenCalledTimes(1);
});

it("keeps the original native domain rejection object untouched in disabled mode", async () => {
  const state = setup(catalog(false));
  const failure = Object.freeze({code: "invalid_input", message: "稿件路径无效"});
  state.invokeCommand.mockRejectedValueOnce(failure);
  await expect(state.preview(sourcePath)).rejects.toBe(failure);
});

it("retains Core catalog failure code, safe cause and original identity without starting a preview", async () => {
  const failure = Object.freeze({code: "runtime_plugin_resource_changed", packageKey, message: "插件包资源已改变。"});
  const invokeCommand = vi.fn<typeof invoke>().mockRejectedValueOnce(failure);
  const pending = createNativeWritingPreview(invokeCommand as typeof invoke)(sourcePath);
  await expect(pending).rejects.toMatchObject({...failure, message: expect.stringContaining(failure.message), cause: failure});
  await expect(pending).rejects.toThrow("runtime_plugin_resource_changed");
  expect(invokeCommand).toHaveBeenCalledTimes(1);
});

it("makes native request and exit identity visible to the unchanged message-only import UI", async () => {
  const state = setup();
  const failure = Object.freeze({code: "plugin_host_exited", message: "原生插件在回复前退出。", pluginId: "writing_projects",
    packageKey, instanceId: "instance-7", requestId: "request-9", exitCode: 23, signal: 9,
    cleanupError: "process I/O failed", privateBody: "never include the raw body"});
  state.invokeCommand.mockRejectedValueOnce(failure);
  const pending = state.preview(sourcePath);
  await expect(pending).rejects.toMatchObject({code: failure.code, pluginId: failure.pluginId, packageKey,
    instanceId: failure.instanceId, requestId: failure.requestId, exitCode: 23, signal: 9,
    cleanupError: failure.cleanupError, cause: failure});
  for (const text of [failure.code, failure.message, "pluginId=writing_projects", "instanceId=instance-7",
    "requestId=request-9", "exitCode=23", "signal=9", "cleanupError=process I/O failed"])
    await expect(pending).rejects.toThrow(text);
  await expect(pending).rejects.not.toThrow(failure.privateBody);
  expect(state.invokeCommand.mock.calls.map(([command]) => command)).toEqual([
    "list_runtime_plugin_packages", "preview_runtime_manuscript_import",
  ]);
});

it("categorizes an unknown transport exception without exporting its arbitrary message", async () => {
  const state = setup();
  const cause = new Error("private document data must stay out of the displayed failure");
  state.invokeCommand.mockRejectedValueOnce(cause);
  const pending = state.preview(sourcePath);
  await expect(pending).rejects.toMatchObject({code: "plugin_host_failed", packageKey, cause});
  await expect(pending).rejects.not.toThrow(cause.message);
  expect(state.invokeCommand).toHaveBeenCalledTimes(2);
});

it("forwards the safe termination diagnostic through the original message-only import error", async () => {
  const state = setup();
  const terminationError = Object.freeze({operation: "group_terminate", kind: "PermissionDenied", rawOsError: 1});
  const failure = Object.freeze({code: "plugin_host_response_mismatch", message: "原生插件回复与本次请求不匹配。",
    pluginId: "writing_projects", packageKey, instanceId: "instance-7", requestId: "request-9", exitCode: 0,
    terminationError, privateBody: "PRIVATE_BODY"});
  state.invokeCommand.mockRejectedValueOnce(failure);
  const pending = state.preview(sourcePath);
  await expect(pending).rejects.toMatchObject({code: failure.code, pluginId: failure.pluginId, packageKey,
    instanceId: failure.instanceId, requestId: failure.requestId, exitCode: 0,
    terminationError, diagnostic: {terminationError}, cause: failure});
  for (const text of [failure.code, failure.message, "pluginId=writing_projects", `packageKey=${packageKey}`,
    "instanceId=instance-7", "requestId=request-9", "exitCode=0", "operation=group_terminate",
    "kind=PermissionDenied", "rawOsError=1", "终止阶段诊断（已确认完成收尾）"])
    await expect(pending).rejects.toThrow(text);
  await expect(pending).rejects.not.toThrow("[object Object]");
  await expect(pending).rejects.not.toThrow(failure.privateBody);
  expect(state.invokeCommand).toHaveBeenCalledTimes(2);
});

it("keeps the exact previous error message when no termination diagnostic exists", async () => {
  const state = setup();
  const failure = Object.freeze({code: "plugin_host_exited", message: "原生插件在回复前退出。", pluginId: "writing_projects",
    packageKey, instanceId: "instance-7", requestId: "request-9", exitCode: 23, signal: 9,
    cleanupError: "process I/O failed"});
  state.invokeCommand.mockRejectedValueOnce(failure);
  await expect(state.preview(sourcePath)).rejects.toHaveProperty("message", [
    `${failure.code}: ${failure.message}`, "pluginId=writing_projects", `packageKey=${packageKey}`,
    "instanceId=instance-7", "requestId=request-9", "exitCode=23", "signal=9", "cleanupError=process I/O failed",
  ].join(" · "));
});

const untrustedTerminations: readonly {readonly terminationError: unknown}[] = [
  {terminationError: {operation: "group_terminate", kind: "PermissionDenied", rawOsError: 1, body: "PRIVATE_TERM_BODY"}},
  {terminationError: {operation: "/PRIVATE_TERM_PATH", kind: "PermissionDenied", rawOsError: 1}},
  {terminationError: {toString: () => "PRIVATE_TERM_STRING"}},
];

it.each(untrustedTerminations)("omits an untrusted termination object while preserving the original rejection", async ({terminationError}) => {
  const state = setup();
  const failure = Object.freeze({code: "plugin_host_exited", message: "原生插件在回复前退出。",
    pluginId: "writing_projects", packageKey, requestId: "request-9", terminationError});
  state.invokeCommand.mockRejectedValueOnce(failure);
  const pending = state.preview(sourcePath);
  await expect(pending).rejects.toMatchObject({code: failure.code, requestId: failure.requestId, cause: failure});
  await expect(pending).rejects.toHaveProperty("terminationError", undefined);
  await expect(pending).rejects.not.toThrow("PRIVATE_TERM");
  await expect(pending).rejects.not.toThrow("[object Object]");
  await expect(pending).rejects.not.toThrow("终止阶段诊断");
});

it("does not accept a termination claim from an unknown transport exception", async () => {
  const state = setup();
  const failure = Object.assign(new Error("PRIVATE_UNKNOWN_MESSAGE"), {
    terminationError: {operation: "group_terminate", kind: "PermissionDenied", rawOsError: 1},
  });
  state.invokeCommand.mockRejectedValueOnce(failure);
  const pending = state.preview(sourcePath);
  await expect(pending).rejects.toMatchObject({code: "plugin_host_failed", cause: failure});
  await expect(pending).rejects.toHaveProperty("terminationError", undefined);
  await expect(pending).rejects.not.toThrow("group_terminate");
  await expect(pending).rejects.not.toThrow(failure.message);
});
