import {invoke} from "../../../core/runtime/platformCore";
import type {LiteratureGateway} from "@research-assistant/literature-library-plugin/contracts/literatureGateway";
import type {AttachmentSummary, AttachmentRemovalResult, DuplicateCandidate, ImportJob, LiteratureItem, MergeOperation, MetadataPreview} from "@research-assistant/core/literature";
import {literatureCommandError as desktopCommandError} from "@research-assistant/literature-library-plugin/transportDecoders";
import {attachmentBytes, readingState, readingStateArguments, type AttachmentByteResponse, type ReadingPositionResponse} from "./literatureAttachmentTransport";

async function invokeLiterature<T>(
  command: string,
  args?: Record<string, unknown>,
): Promise<T> {
  try {
    return await invoke<T>(command, args);
  } catch (reason: unknown) {
    throw desktopCommandError(reason);
  }
}

export const tauriLiteratureGateway: LiteratureGateway = {
  async list(query) {
    return invoke<readonly LiteratureItem[]>("list_literature", { query });
  },
  async listTrash(query) {
    return invoke<readonly LiteratureItem[]>("list_trashed_literature", { query });
  },
  async get(id) {
    return invoke<LiteratureItem>("get_literature_item", { id });
  },
  async save(draft) {
    return invoke<LiteratureItem>("save_literature_item", { draft });
  },
  async updateMetadataField(itemId, patch) {
    return invokeLiterature<LiteratureItem>("update_literature_metadata_field", { itemId, patch });
  },
  async importPdfs(candidates) {
    return invoke<ImportJob>("start_pdf_import", { candidates });
  },
  async importBibtex(drafts) {
    return invoke<ImportJob>("import_bibtex_drafts", { drafts });
  },
  async getImportJob(id) {
    return invoke<ImportJob>("get_import_job", { id });
  },
  async listImportJobs() {
    return invoke<readonly ImportJob[]>("list_import_jobs");
  },
  async cancelImportJob(id) {
    await invoke("cancel_import_job", { id });
  },
  async attachPdf(itemId, path) {
    return invokeLiterature<AttachmentSummary>("attach_pdf_path", { itemId, path });
  },
  async renameAttachment(itemId, attachmentId, displayName) {
    await invokeLiterature("rename_literature_attachment", {itemId, attachmentId, displayName});
  },
  async removeAttachment(itemId, attachmentId) {
    return invokeLiterature<AttachmentRemovalResult>("remove_literature_attachment", {itemId, attachmentId});
  },
  async previewDoi(doi, itemId) {
    return invokeLiterature<MetadataPreview>("preview_doi_metadata", { doi, itemId });
  },
  async applyMetadataCandidate(candidateId, draft) {
    return invoke<LiteratureItem>("apply_metadata_candidate", { candidateId, draft });
  },
  async applyMetadataCandidateFields(candidateId, decisions, candidateTrust) {
    return invokeLiterature<LiteratureItem>("apply_metadata_candidate_fields", {
      candidateId,
      decisions,
      candidateTrust,
    });
  },
  async rejectMetadataCandidate(candidateId) {
    await invokeLiterature("reject_metadata_candidate", { candidateId });
  },
  async readAttachment(attachmentId) {
    const value = await invoke<AttachmentByteResponse>(
      "read_attachment",
      { attachmentId },
    );
    return attachmentBytes(value);
  },
  async revealAttachments(attachmentIds) {
    try {
      await invoke("reveal_attachments_in_file_manager", { attachmentIds });
    } catch (reason: unknown) {
      throw desktopCommandError(reason);
    }
  },
  async saveReadingState(itemId, attachmentId, viewState) {
    await invoke("save_reading_state", {
      itemId,
      attachmentId,
      ...readingStateArguments(viewState),
    });
  },
  async getReadingState(itemId, attachmentId) {
    const value = await invoke<ReadingPositionResponse | null>("get_reading_state", { itemId, attachmentId });
    return readingState(value);
  },
  async listDuplicates() {
    return invoke<readonly DuplicateCandidate[]>("list_duplicate_candidates");
  },
  async decideDuplicate(candidateId, decision) {
    await invoke("decide_duplicate_candidate", { candidateId, decision });
  },
  async mergeDuplicate(candidateId, survivorItemId) {
    return invoke<MergeOperation>("merge_duplicate_candidate", { candidateId, survivorItemId });
  },
  async listMergeOperations() {
    return invoke<readonly MergeOperation[]>("list_merge_operations");
  },
  async undoMerge(operationId) {
    await invoke("undo_merge_operation", { operationId });
  },
  async confirmMetadataFields(itemId, fieldNames) {
    return invokeLiterature<LiteratureItem>("confirm_literature_metadata_fields", { itemId, fieldNames });
  },
  async setProjects(itemId, projectIds) {
    await invoke("set_item_projects", { itemId, projectIds });
  },
  async archive(itemId) {
    await invoke("archive_literature_item", { itemId });
  },
  async restore(itemId) {
    await invoke("restore_literature_item", { itemId });
  },
  async trash(itemId) {
    await invoke("trash_literature_item", { itemId });
  },
  async trashMany(itemIds) {
    await invokeLiterature("trash_literature_items", { itemIds });
  },
  async restoreTrash(itemId) {
    await invoke("restore_trashed_literature_item", { itemId });
  },
  async isAttachmentIndexed(attachmentId) {
    return invoke<boolean>("attachment_is_indexed", { attachmentId });
  },
  async indexAttachment(itemId, attachmentId, pages) {
    await invoke("index_attachment_text", { itemId, attachmentId, pages });
  },
};
