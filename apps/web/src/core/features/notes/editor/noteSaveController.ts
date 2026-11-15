import type { NoteDocumentV1, NotePageId } from "@research-assistant/core/notes";
import { NotesGatewayError, type NoteDocumentSnapshot, type NotesGateway } from "@research-assistant/data/notes";
import type { NoteExternalSaveLease } from "./noteExternalSaveLease";
import { EDITOR_AUTOSAVE_IDLE_MS } from "../../../services/editorAutosave";

import type {NoteSaveStatus} from "@research-assistant/workbench-sdk/note-documents";
export type {NoteSaveStatus, NoteSaveCoordinator} from "@research-assistant/workbench-sdk/note-documents";

interface NoteSaveControllerOptions {
  readonly pageId: NotePageId;
  readonly gateway: NotesGateway;
  readonly initialRevision: number;
  readonly debounceMs?: number;
  readonly onSnapshot: (snapshot: NoteDocumentSnapshot) => void;
  readonly onDurable?: (snapshot: NoteDocumentSnapshot) => void;
  readonly onStatus: (status: NoteSaveStatus, message?: string) => void;
}

export class NoteSaveController {
  private readonly pageId: NotePageId;
  private readonly gateway: NotesGateway;
  private readonly debounceMs: number;
  private readonly onSnapshot: NoteSaveControllerOptions["onSnapshot"];
  private readonly onDurable: NoteSaveControllerOptions["onDurable"];
  private readonly onStatus: NoteSaveControllerOptions["onStatus"];
  private revision: number;
  private pending: NoteDocumentV1 | undefined;
  private timer: ReturnType<typeof setTimeout> | undefined;
  private inFlight: Promise<void> | undefined;
  private isDisposed = false;
  private blockedBy: "failed" | "conflict" | undefined;
  private external: { readonly token: symbol; dispatched: boolean; invalidated: boolean } | undefined;

  constructor(options: NoteSaveControllerOptions) {
    this.pageId = options.pageId;
    this.gateway = options.gateway;
    this.revision = options.initialRevision;
    this.debounceMs = options.debounceMs ?? EDITOR_AUTOSAVE_IDLE_MS;
    this.onSnapshot = options.onSnapshot;
    this.onDurable = options.onDurable;
    this.onStatus = options.onStatus;
  }

  enqueue(document: NoteDocumentV1): void {
    if (this.isDisposed) return;
    this.pending = document;
    if (this.external !== undefined) {
      this.external.invalidated = true;
      this.blockedBy = "conflict";
      this.onStatus("conflict", "外部修改期间出现了新的编辑，请核对版本后再保存");
      return;
    }
    if (this.blockedBy === "conflict") {
      this.onStatus("conflict");
      return;
    }
    this.blockedBy = undefined;
    this.onStatus("dirty");
    if (this.timer !== undefined) clearTimeout(this.timer);
    this.timer = setTimeout(() => {
      this.timer = undefined;
      void this.flushPending(false);
    }, this.debounceMs);
  }

  async flush(): Promise<void> {
    await this.flushPending(true);
  }

  private async flushPending(immediate: boolean): Promise<void> {
    if (this.isDisposed || this.external !== undefined) return;
    if (immediate && this.timer !== undefined) {
      clearTimeout(this.timer);
      this.timer = undefined;
    }
    if (this.inFlight !== undefined) {
      await this.inFlight;
      if (this.pending !== undefined && this.blockedBy === undefined && (immediate || this.timer === undefined)) await this.flushPending(immediate);
      return;
    }
    if (this.blockedBy !== undefined) {
      this.onStatus(this.blockedBy);
      return;
    }
    const document = this.pending;
    if (document === undefined) return;
    this.pending = undefined;
    this.onStatus("saving");
    this.inFlight = this.save(document);
    await this.inFlight;
    this.inFlight = undefined;
    if (this.pending !== undefined && !this.isDisposed && this.blockedBy === undefined && (immediate || this.timer === undefined)) await this.flushPending(immediate);
  }

  private async save(document: NoteDocumentV1): Promise<void> {
    try {
      const snapshot = await this.gateway.saveDocument({
        document,
        expectedRevision: this.revision,
        saveReason: "autosave",
      });
      this.revision = snapshot.page.revision;
      if (!this.isDisposed) {
        this.onSnapshot(this.pending === undefined ? snapshot : { ...snapshot, document: this.pending });
        if (this.pending === undefined) {
          this.onDurable?.(snapshot);
          this.onStatus("saved");
        }
      }
    } catch (reason: unknown) {
      this.pending ??= document;
      if (reason instanceof NotesGatewayError && reason.code === "revision_conflict") {
        this.blockedBy = "conflict";
        this.onStatus("conflict", reason.message);
      } else {
        this.blockedBy = "failed";
        this.onStatus("failed", reason instanceof Error ? reason.message : String(reason));
      }
    }
  }

  updateRevision(revision: number): void {
    if (this.isDisposed) return;
    this.revision = revision;
    if (this.external !== undefined) {
      this.external.invalidated = true;
      this.blockedBy = "conflict";
      this.onStatus("conflict", "笔记版本发生变化，请先核对外部修改结果");
      return;
    }
    this.blockedBy = undefined;
  }

  restore(document: NoteDocumentV1, state: Extract<NoteSaveStatus, "dirty" | "conflict">): void {
    if (this.isDisposed) return;
    if (this.external !== undefined) { this.enqueue(document); return; }
    this.pending = document;
    this.blockedBy = state === "conflict" ? "conflict" : undefined;
    this.onStatus(state, state === "conflict" ? "恢复草稿基于较早版本，请比较后再保存" : undefined);
    if (state === "dirty") {
      if (this.timer !== undefined) clearTimeout(this.timer);
      this.timer = setTimeout(() => {
        this.timer = undefined;
        void this.flushPending(false);
      }, this.debounceMs);
    }
  }

  hasUnsavedChanges(): boolean {
    return this.pending !== undefined
      || this.timer !== undefined
      || this.inFlight !== undefined
      || this.external !== undefined
      || this.blockedBy !== undefined;
  }

  hasExternalChange(): boolean { return this.external !== undefined; }

  async retry(): Promise<void> {
    if (this.isDisposed || this.external !== undefined) return;
    this.blockedBy = undefined;
    await this.flush();
  }

  dispose(): void {
    this.isDisposed = true;
    this.external = undefined;
    if (this.timer !== undefined) clearTimeout(this.timer);
    this.timer = undefined;
  }

  /** Does not flush pending edits: a proposal is tied to the reviewed saved
   * revision, and silently saving newer input would change that baseline. */
  beginExternalChange(expectedRevision: number, baseline: NoteDocumentV1): NoteExternalSaveLease | null {
    if (this.isDisposed || baseline.pageId !== this.pageId || this.hasUnsavedChanges() || this.revision !== expectedRevision
      || !Number.isSafeInteger(expectedRevision) || expectedRevision < 0 || expectedRevision >= Number.MAX_SAFE_INTEGER) return null;
    const owner = {token: Symbol("note-external-change"), dispatched: false, invalidated: false};
    const pageId = baseline.pageId;
    this.external = owner;
    const current = () => !this.isDisposed && this.external?.token === owner.token;
    const requireRecovery = () => {
      if (!current()) return;
      this.blockedBy = "conflict";
      this.onStatus("conflict", "外部修改结果尚未核对，请恢复原操作后再保存");
    };
    this.onStatus("saving");
    const commit = (snapshot: NoteDocumentSnapshot, appliedRevision: number, installEditor: () => undefined): boolean => {
      if (!current() || !owner.dispatched || owner.invalidated || this.pending !== undefined
        || this.revision !== expectedRevision || snapshot.page.id !== pageId || snapshot.document.pageId !== pageId
        || !Number.isSafeInteger(appliedRevision) || appliedRevision <= expectedRevision || snapshot.page.revision !== appliedRevision) return false;
      try { if (installEditor() !== undefined) { requireRecovery(); return false; } }
      catch (reason) { requireRecovery(); throw reason; }
      if (!current() || owner.invalidated || this.pending !== undefined) { requireRecovery(); return false; }
      this.revision = snapshot.page.revision; this.external = undefined; this.blockedBy = undefined;
      this.onSnapshot(snapshot); this.onDurable?.(snapshot); this.onStatus("saved");return true;
    };
    return {
      markDispatched: () => {
        if (!current() || owner.dispatched || owner.invalidated) return false;
        owner.dispatched = true; return true;
      },
      cancelBeforeDispatch: () => {
        if (!current() || owner.dispatched) return false;
        this.external = undefined;
        if (owner.invalidated || this.pending !== undefined) {
          this.blockedBy = "conflict"; this.onStatus("conflict");
        } else { this.blockedBy = undefined; this.onStatus("saved"); }
        return true;
      },
      rejectDispatched: () => {
        if (!current() || owner.invalidated || this.pending !== undefined) return false;
        this.external = undefined; this.blockedBy = undefined; this.onStatus("saved"); return true;
      },
      commit: (snapshot, installEditor) => commit(snapshot, expectedRevision + 1, installEditor),
      commitGroup: commit,
      requireRecovery,
    };
  }
}
