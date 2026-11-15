import {beforeEach, expect, it, vi} from "vitest";
import {createStableCitationKey} from "@research-assistant/writing";
import {tauriWritingGateway} from "./tauriWritingGateway";
const platform = vi.hoisted(() => ({invoke: vi.fn()}));
vi.mock("../../../core/runtime/platformCore", () => ({invoke: platform.invoke}));
beforeEach(() => platform.invoke.mockReset());

it("retains the nullable session and latest-compile compatibility conversions", async () => {
  for (const [read, command] of [
    [() => tauriWritingGateway.getSession("manuscript"), "get_writing_session"],
    [() => tauriWritingGateway.getLatestCompileRun("manuscript"), "get_latest_manuscript_compile_run"],
    [() => tauriWritingGateway.getLatestPdfCompileRun("manuscript"), "get_latest_pdf_manuscript_compile_run"],
  ] as const) {
    platform.invoke.mockResolvedValueOnce(null);
    await expect(read()).resolves.toBeUndefined();
    expect(platform.invoke).toHaveBeenLastCalledWith(command, {manuscriptId: "manuscript"});
    const receipt = {synthetic: "original receipt"};
    platform.invoke.mockResolvedValueOnce(receipt);
    await expect(read()).resolves.toBe(receipt);
  }
});

it("retains PDF byte conversion and original compilation request arguments", async () => {
  platform.invoke.mockResolvedValueOnce([0, 255, 42]);
  await expect(tauriWritingGateway.readCompilePdf("run")).resolves.toEqual(new Uint8Array([0, 255, 42]));
  expect(platform.invoke).toHaveBeenLastCalledWith("read_manuscript_compile_pdf", {runId: "run"});
  const receipt = {id: "run"}; platform.invoke.mockResolvedValue(receipt);
  await expect(tauriWritingGateway.startCompile("manuscript", "xelatex")).resolves.toBe(receipt);
  expect(platform.invoke).toHaveBeenLastCalledWith("start_manuscript_compile", {manuscriptId: "manuscript", engine: "xelatex", options: undefined});
  const options = {draft: true, stopOnFirstError: false, fromScratch: true};
  await tauriWritingGateway.startCompile("manuscript", "xelatex", options);
  expect(platform.invoke).toHaveBeenLastCalledWith("start_manuscript_compile", {manuscriptId: "manuscript", engine: "xelatex", options});
});

it("preserves existing defaults and export conflict policy", async () => {
  platform.invoke.mockResolvedValue({});
  await tauriWritingGateway.createFile("manuscript", "main.tex");
  expect(platform.invoke).toHaveBeenLastCalledWith("create_manuscript_file", {manuscriptId: "manuscript", relativePath: "main.tex", content: ""});
  await tauriWritingGateway.listBibliographyEntries("manuscript");
  expect(platform.invoke).toHaveBeenLastCalledWith("list_manuscript_bibliography_entries", {manuscriptId: "manuscript", includeUnusedAutomatic: false});
  await tauriWritingGateway.exportSource("manuscript", "/destination");
  expect(platform.invoke).toHaveBeenLastCalledWith("export_manuscript_source", {manuscriptId: "manuscript", destinationPath: "/destination", renameOnConflict: true});
});

it("normalizes native citation nulls while preserving candidates and query-only behavior", async () => {
  const fields = {firstAuthor: "Example", containerTitle: "Synthetic Journal", importedKey: "",
    hasPdf: true, hasEvidence: false, warnings: Object.freeze(["synthetic warning"])};
  const nullable = Object.freeze({...fields, literatureItemId: "nullable", title: "Undated study", issuedYear: null, citationKey: null});
  const missing = Object.freeze({...fields, literatureItemId: "missing", title: "Missing fields"});
  const existing = Object.freeze({...fields, literatureItemId: "bound", title: "Bound study", issuedYear: 2026, citationKey: "PinnedKey"});
  const nativeCandidates = Object.freeze([nullable, missing, existing]);
  const original = structuredClone(nativeCandidates);
  platform.invoke.mockResolvedValueOnce(nativeCandidates);

  const candidates = await tauriWritingGateway.searchCitations("manuscript", "  study  ", "library");

  expect(platform.invoke).toHaveBeenCalledExactlyOnceWith("search_manuscript_citations", {manuscriptId: "manuscript", query: "  study  ", scope: "library"});
  expect(candidates).toStrictEqual([
    {...fields, literatureItemId: "nullable", title: "Undated study"},
    missing,
    existing,
  ]);
  expect(nativeCandidates).toStrictEqual(original);
  const occupied = new Set(candidates.flatMap(candidate => candidate.citationKey === undefined ? [] : [candidate.citationKey]));
  expect(candidates.map(candidate => candidate.citationKey ?? createStableCitationKey({
    importedKey: candidate.importedKey, firstAuthor: candidate.firstAuthor, title: candidate.title,
    ...(candidate.issuedYear === undefined ? {} : {issuedYear: candidate.issuedYear}),
  }, occupied))).toEqual(["exampleundated", "examplemissing", "PinnedKey"]);

  const failure = {code: "citation_search_unavailable", taskId: "task-8", message: "safe native cause"};
  platform.invoke.mockRejectedValueOnce(failure);
  await expect(tauriWritingGateway.searchCitations("manuscript", "  study  ", "library")).rejects.toBe(failure);
  expect(platform.invoke.mock.calls.map(([command]) => command)).toEqual(["search_manuscript_citations", "search_manuscript_citations"]);
});

it("propagates the original native failure and request identity", async () => {
  const failure = {code: "compile_unavailable", taskId: "task-7", message: "safe native cause"};
  platform.invoke.mockRejectedValueOnce(failure);
  await expect(tauriWritingGateway.getCompileRun("run")).rejects.toBe(failure);
  expect(platform.invoke).toHaveBeenCalledExactlyOnceWith("get_manuscript_compile_run", {runId: "run"});
});
