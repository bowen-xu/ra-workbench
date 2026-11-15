import { invoke as invokePlatform } from "../../../core/runtime/platformCore";
import type {
  Manuscript,
  ManuscriptDraft,
  ManuscriptFile,
  ManuscriptFileContent,
  SaveManuscriptFileRequest,
  WritingSessionDraft,
  CompileRun,
  CompileOptions,
  LatexEngine,
  LatexToolchainInfo,
  BibliographyEntryInput,
  BibliographyStatus,
  CitationBinding,
  CitationCandidate,
  CitationScope,
  RecoveryDraft,
  RecoveryDraftInput,
  UpdateManuscriptRequest,
  WritingSession,
  ManuscriptTransferResult,
  ManuscriptExportResult,
  SynctexPdfLocation,
  SynctexSourceLocation,
  ManuscriptDirectory,
  ManuscriptResourceImportResult,
} from "@research-assistant/writing";

import type { WritingGateway } from "@research-assistant/data/writing";
import {createTauriWritingNoteCatalogGateway, tauriWritingNoteCatalogGateway} from "./tauriWritingNoteCatalogGateway";
import {previewWritingImport} from "./nativeWritingPreview";

type NativeCitationCandidate = Omit<CitationCandidate, "issuedYear" | "citationKey"> & {
  readonly issuedYear?: number | null;
  readonly citationKey?: string | null;
};

/** Static and package-bound assembly share the original method and codec implementation. */
export function createTauriWritingGateway(
  invoke: typeof invokePlatform = invokePlatform,
  previewImport: WritingGateway["previewImport"] = previewWritingImport,
): WritingGateway {
  return {
    noteCatalog: invoke === invokePlatform ? tauriWritingNoteCatalogGateway : createTauriWritingNoteCatalogGateway(invoke),
    listBibliographyEntries: (manuscriptId, includeUnusedAutomatic = false) => invoke("list_manuscript_bibliography_entries", { manuscriptId, includeUnusedAutomatic }),
    removeUnusedImportedBibliographyEntries: manuscriptId => invoke("remove_unused_imported_bibliography_entries", {manuscriptId}),
    linkBibliographyEntry: (manuscriptId, entryId, literatureItemId) => invoke("link_manuscript_bibliography_entry", { manuscriptId, entryId, literatureItemId }),
    updateBibliographyEntry: (manuscriptId, entryId, expectedBibtex, draft) => invoke("update_manuscript_bibliography_entry", { manuscriptId, entryId, expectedBibtex, draft }),
    supportsCompilation: true,
    duplicateManuscript: manuscriptId => invoke<Manuscript>("duplicate_manuscript", {manuscriptId}),
    async listManuscripts() {
      return invoke<readonly Manuscript[]>("list_manuscripts");
    },
    async createManuscript(draft: ManuscriptDraft) {
      return invoke<Manuscript>("create_manuscript", { draft });
    },
    async archiveManuscript(manuscriptId) {
      return invoke<Manuscript>("archive_manuscript", { manuscriptId });
    },
    async deleteManuscript(manuscriptId) {
      await invoke("delete_manuscript", { manuscriptId });
    },
    async restoreManuscript(manuscriptId) {
      return invoke<Manuscript>("restore_manuscript", { manuscriptId });
    },
    async updateManuscript(request: UpdateManuscriptRequest) {
      return invoke<Manuscript>("update_manuscript", { request });
    },
    async listFiles(manuscriptId) {
      return invoke<readonly ManuscriptFile[]>("list_manuscript_files", { manuscriptId });
    },
    async readFile(manuscriptId, relativePath) {
      return invoke<ManuscriptFileContent>("read_manuscript_file", { manuscriptId, relativePath });
    },
    async saveFile(request: SaveManuscriptFileRequest) {
      return invoke<ManuscriptFileContent>("save_manuscript_file", { request });
    },
    async createFile(manuscriptId: string, relativePath: string, content = "") {
      return invoke<ManuscriptFileContent>("create_manuscript_file", { manuscriptId, relativePath, content });
    },
    async renameFile(manuscriptId: string, fromPath: string, toPath: string) {
      return invoke<Manuscript>("rename_manuscript_file", { manuscriptId, fromPath, toPath });
    },
    async deleteFile(manuscriptId: string, relativePath: string) {
      await invoke("delete_manuscript_file", { manuscriptId, relativePath });
    },
    async listDirectories(manuscriptId: string) {
      return invoke<readonly ManuscriptDirectory[]>("list_manuscript_directories", { manuscriptId });
    },
    async createDirectory(manuscriptId: string, relativePath: string) {
      return invoke<ManuscriptDirectory>("create_manuscript_directory", { manuscriptId, relativePath });
    },
    async renameDirectory(manuscriptId: string, fromPath: string, toPath: string) {
      return invoke<Manuscript>("rename_manuscript_directory", { manuscriptId, fromPath, toPath });
    },
    async deleteDirectory(manuscriptId: string, relativePath: string) {
      await invoke("delete_manuscript_directory", { manuscriptId, relativePath });
    },
    async importResources(manuscriptId: string, destinationDirectory: string, sourcePaths: readonly string[]) {
      return invoke<ManuscriptResourceImportResult>("import_manuscript_resources", { manuscriptId, destinationDirectory, sourcePaths });
    },
    async saveSession(draft: WritingSessionDraft) {
      await invoke("save_writing_session", { draft });
    },
    async getSession(manuscriptId: string) {
      return (await invoke<WritingSession | null>("get_writing_session", { manuscriptId })) ?? undefined;
    },
    async saveRecoveryDraft(draft: RecoveryDraftInput) {
      return invoke<RecoveryDraft>("save_manuscript_recovery_draft", { draft });
    },
    async listRecoveryDrafts(manuscriptId: string) {
      return invoke<readonly RecoveryDraft[]>("list_manuscript_recovery_drafts", { manuscriptId });
    },
    async clearRecoveryDraft(manuscriptId: string, relativePath: string) {
      await invoke("clear_manuscript_recovery_draft", { manuscriptId, relativePath });
    },
    async previewImport(sourcePath: string) {
      return previewImport(sourcePath);
    },
    async importProject(title: string, sourcePath: string, rootPath: string) {
      return invoke<ManuscriptTransferResult>("import_manuscript_project", { title, sourcePath, rootPath });
    },
    async exportSource(manuscriptId: string, destinationPath: string) {
      return invoke<ManuscriptExportResult>("export_manuscript_source", { manuscriptId, destinationPath, renameOnConflict: true });
    },
    async exportPdf(manuscriptId: string, destinationPath: string) {
      return invoke<ManuscriptExportResult>("export_manuscript_pdf", { manuscriptId, destinationPath, renameOnConflict: true });
    },
    async forwardSynctex(runId: string, relativePath: string, line: number, column: number) {
      return invoke<readonly SynctexPdfLocation[]>("forward_manuscript_synctex", { runId, relativePath, line, column });
    },
    async inverseSynctex(runId: string, page: number, x: number, y: number) {
      return invoke<readonly SynctexSourceLocation[]>("inverse_manuscript_synctex", { runId, page, x, y });
    },
    async detectToolchain() {
      return invoke<LatexToolchainInfo>("detect_latex_toolchain");
    },
    async startCompile(manuscriptId: string, engine: LatexEngine, options?: CompileOptions) {
      return invoke<CompileRun>("start_manuscript_compile", { manuscriptId, engine, options });
    },
    async getCompileRun(runId: string) {
      return invoke<CompileRun>("get_manuscript_compile_run", { runId });
    },
    async getLatestCompileRun(manuscriptId: string) {
      return (await invoke<CompileRun | null>("get_latest_manuscript_compile_run", { manuscriptId })) ?? undefined;
    },
    async getLatestPdfCompileRun(manuscriptId: string) {
      return (await invoke<CompileRun | null>("get_latest_pdf_manuscript_compile_run", { manuscriptId })) ?? undefined;
    },
    async isCompileCurrent(runId: string) {
      return invoke<boolean>("is_manuscript_compile_current", { runId });
    },
    async cancelCompile(runId: string) {
      await invoke("cancel_manuscript_compile", { runId });
    },
    async readCompilePdf(runId: string) {
      return new Uint8Array(await invoke<number[]>("read_manuscript_compile_pdf", { runId }));
    },
    async searchCitations(manuscriptId: string, query: string, scope: CitationScope) {
      const candidates = await invoke<readonly NativeCitationCandidate[]>("search_manuscript_citations", { manuscriptId, query, scope });
      return candidates.map(({ issuedYear, citationKey, ...candidate }) => ({
        ...candidate,
        ...(issuedYear === null || issuedYear === undefined ? {} : { issuedYear }),
        ...(citationKey === null || citationKey === undefined ? {} : { citationKey }),
      }));
    },
    async listCitationBindings(manuscriptId: string) {
      return invoke<readonly CitationBinding[]>("list_manuscript_citation_bindings", { manuscriptId });
    },
    async bindCitation(manuscriptId: string, entry: BibliographyEntryInput) {
      return invoke<CitationBinding>("bind_manuscript_citation", { manuscriptId, entry });
    },
    rekeyCitations: (manuscriptId, assignments) => invoke("rekey_manuscript_citations", {manuscriptId, assignments}),
    async syncBibliography(manuscriptId: string, entries: readonly BibliographyEntryInput[]) {
      return invoke<BibliographyStatus>("sync_manuscript_bibliography", { manuscriptId, entries });
    },
    async getBibliographyStatus(manuscriptId: string) {
      return invoke<BibliographyStatus>("get_manuscript_bibliography_status", { manuscriptId });
    },
    async regenerateBibliography(manuscriptId: string) {
      return invoke<BibliographyStatus>("regenerate_manuscript_bibliography", { manuscriptId });
    },
    async preserveBibliographyEdits(manuscriptId: string) {
      return invoke<BibliographyStatus>("preserve_manuscript_bibliography_edits", { manuscriptId });
    },
  };
}

export const tauriWritingGateway = createTauriWritingGateway();
