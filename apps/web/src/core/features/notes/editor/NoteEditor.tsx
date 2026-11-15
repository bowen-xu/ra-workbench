import { NoteEditorSurface } from "../../../components/block-editor/NoteEditorSurface";
import { chooseAppFile } from "../../../runtime/platformFileInput";
import { useEffect, useMemo, useRef, useState } from "react";
import { useCreateBlockNote } from "@blocknote/react";
import { LinkToolbarController } from "../../../components/block-editor/LinkToolbarController";

import { isAllowedNoteHref, type NoteBlockV1, type NoteDocumentV1 } from "@research-assistant/core/notes";
import { NotesGatewayError,type NoteTargetQuery } from "@research-assistant/data/notes";
import {catalogErrorMessage} from "../noteCatalogError";

import { useNotesClient } from "../NotesClient";
import {reportNoteCreationError} from "../workspace/NotePages";
import type { ResearchObjectTarget } from "../links/useResearchObjectTargets";
import { blockNoteToNoteDocument, noteDocumentToBlockNote, retainNoteProperties } from "./blockNoteAdapter";
import { createNoteEditorInstallPort, type NoteEditorInstallPort } from "./noteEditorInstallPort";
import { registerTextInstallPort, useTextEditorHeld, isTextEditorHeld, flushNoteReview } from "./noteTextEditorPorts";
import { blockEditorDefaults } from "../../../components/block-editor";
import { noteBlockMovePolicy } from "./noteBlockMovePolicy";
import { noteEditorSchema, type NoteEditorInstance } from "./noteEditorSchema";
import { buildNoteDomainSlashItems, buildResearchMentionItems, filterNoteSuggestionItems } from "./noteSlashMenu";
import { closeHistory } from "@tiptap/pm/history";
import { notify } from "../../../services/notifications";
import { NoteEvidenceMenu } from "./NoteEvidenceMenu";
import { NoteSelectionAskAi } from "./NoteSelectionAskAi";
import { NoteClipboard } from "./NoteClipboard";
import {ManagedAssetIngress, type AssetInsertion} from "../../../components/block-editor/ManagedAssetIngress";
import { noteEmptyBlockPresentation } from "./noteEmptyBlockPresentation";
import { MentionSuggestionMenu } from "../../../components/block-editor/SynchronizedSuggestionMenu";
import { EditorSuggestionMenu } from "../../../components/block-editor/EditorSuggestionMenu";
import { createExtension } from "@blocknote/core";
import { createNoteReviewHistory } from "./noteReviewHistory";
import { NoteEditorDiff } from "./NoteEditorDiff";
import { NoteReviewRecoveryControl } from "./NoteReviewRecoveryControl";
import { useNoteTextReview } from "./useNoteTextReview";
import { noteReviewDocumentFromState, synchronizeNoteReviewBaseline } from "./noteReviewProjection";
import {openSourceLink} from "../../agent/sourceNavigation";
import { NoteLinkToolbar } from "./NoteLinkToolbar";
import { noteEditorComponents } from "./noteLinkComponents";
import type { NoteEditorSession } from "./noteEditorSession";
import { NotePageLinkPicker } from "./NotePageLinkPicker";
import { EditorDiffHighlight, type EditorDiffHighlightAppearance } from "../../../components/editor-diff/EditorDiffHighlight";
import {documentTitleBlock, documentTitleExtension, type DocumentTitleBinding} from "../../../components/block-editor/documentTitle";
import {noteCommandHistoryPlugin} from "./noteCommandHistoryPlugin";
import {useNoteHistoryScope} from "./useNoteHistoryScope";

export interface NoteEditorProps {
  readonly autoFocus?: boolean;
  readonly pageTitle?: DocumentTitleBinding | undefined;
  readonly diffHighlight?: EditorDiffHighlightAppearance;
  readonly session?: NoteEditorSession;
  readonly onEditorReady?: (editor: NoteEditorInstance) => void;
  readonly diffControlsTarget?: HTMLElement | null;
  readonly sourceNavigation?: import("../../../contracts/sourceNavigation").NoteSourceNavigation | undefined;
  readonly document: NoteDocumentV1;
  readonly isReadOnly?: boolean;
  readonly conflictLocked?: boolean;
  readonly onChange: (document: NoteDocumentV1) => void;
  readonly pageTargets?: readonly { readonly id: string; readonly title: string }[];
  readonly readingTargets?: readonly { readonly id: string; readonly label: string; readonly quote: string }[];
  readonly queryNoteTargets?: NoteTargetQuery|undefined;
  readonly researchTargets?: readonly ResearchObjectTarget[];
  readonly onAssetStatus?: (message: string) => void;
  readonly onInstallPort?: (port: NoteEditorInstallPort | undefined) => void;
}

export function NoteEditor(props: NoteEditorProps): React.JSX.Element {
  return <EditorDiffHighlight appearance={props.diffHighlight}><NoteEditorInstanceView key={props.session?.key ?? "native-note-review"} {...props}/></EditorDiffHighlight>;
}
function NoteEditorInstanceView({
  document,
  isReadOnly = false,
  conflictLocked = false,
  onChange,
  pageTargets = [],
  readingTargets = [],
  queryNoteTargets,
  researchTargets = [],
  onAssetStatus,
  onInstallPort,
  sourceNavigation,
  diffControlsTarget,
  session,
  onEditorReady,
  pageTitle,
  autoFocus=false,
}: NoteEditorProps): React.JSX.Element {
  const notesClient = useNotesClient();
  const textHeld = useTextEditorHeld(document.pageId);
  const heldRef = useRef(textHeld); heldRef.current = textHeld;
  const alive = useRef(true);
  useEffect(() => { alive.current = true; return () => { alive.current = false; }; }, []);
  const fileInputRef = useRef<HTMLInputElement>(null);
  const assetCancelRequestedRef = useRef(false);
  const assetImportRunningRef = useRef(false);
  const assetBatchRunningRef = useRef(false);
  const composingRef = useRef(false);
  const conflictRef = useRef(conflictLocked); conflictRef.current = conflictLocked;
  const readOnlyRef = useRef(isReadOnly);
  const reviewLocked = useRef(false);
  readOnlyRef.current = isReadOnly;
  const installPortRef = useRef<NoteEditorInstallPort | undefined>(undefined);
  const reviewPublisher = useRef<() => void>(() => {});
  const canReplayReviewHistory = useRef<() => boolean>(() => true);
  const trustedDocument = useRef(document); trustedDocument.current = document;
  const reviewPendingIds = useRef<readonly string[]>([]);
  const [isImportingAsset, setIsImportingAsset] = useState(false);
  const [pageLinkBlockId, setPageLinkBlockId] = useState<string>();
  const titleEnabled = useRef(pageTitle !== undefined);

  const importAsset = async (file: File) => {
    if (assetImportRunningRef.current) throw new NotesGatewayError("transfer_failed", "已有附件正在导入");
    assetImportRunningRef.current = true;
    assetCancelRequestedRef.current = false;
    setIsImportingAsset(true);
    try {
      const bytes = [...new Uint8Array(await file.arrayBuffer())];
      if (assetCancelRequestedRef.current) throw new NotesGatewayError("operation_cancelled", "附件导入已取消");
      const asset = await notesClient.mutate(async (gateway) => gateway.importNoteAsset({
        displayName: file.type.startsWith("image/") ? "image" : file.name,
        mimeType: file.type || "application/octet-stream",
        bytes,
      }));
      if (assetCancelRequestedRef.current) throw new NotesGatewayError("operation_cancelled", "附件导入已取消");
      const message = `已导入附件：${asset.displayName}`;
      if (alive.current) {
        if (onAssetStatus) onAssetStatus(message); else notify.success(message);
      }
      return asset;
    } catch (reason: unknown) {
      if (reason instanceof NotesGatewayError && reason.code === "operation_cancelled") {
        const message = "附件导入已取消";
        if (alive.current) {
          if (onAssetStatus) onAssetStatus(message); else notify.info(message);
        }
      } else {
        notify.error(reason instanceof Error ? reason.message : String(reason));
      }
      throw reason;
    } finally {
      assetCancelRequestedRef.current = false;
      assetImportRunningRef.current = false;
      if (alive.current) setIsImportingAsset(false);
    }
  };

  const initialContent = useMemo(() => {
    const body = noteDocumentToBlockNote(document);
    return titleEnabled.current ? [documentTitleBlock(document.pageId, document.title ?? pageTitle?.value ?? ""), ...body] : body;
  }, [document]);
  const editor:NoteEditorInstance = useCreateBlockNote({
    ...blockEditorDefaults,
    schema: noteEditorSchema,
    links: { ...blockEditorDefaults.links, isValidLink: isAllowedNoteHref },
    extensions: [...(titleEnabled.current ? [documentTitleExtension(document.pageId)] : []), noteEmptyBlockPresentation(), createExtension(() => ({ key: "note-review-history-extension",
      prosemirrorPlugins: [...(session?.createHistory(document, () => !readOnlyRef.current && !conflictRef.current && !heldRef.current && !isTextEditorHeld(document.pageId) && !reviewLocked.current && canReplayReviewHistory.current()) ?? [createNoteReviewHistory(document, () => reviewPublisher.current(), (before, after, baseline) =>
        synchronizeNoteReviewBaseline(baseline,
          noteReviewDocumentFromState(document.pageId, before.doc, trustedDocument.current),
          noteReviewDocumentFromState(document.pageId, after.doc, trustedDocument.current), reviewPendingIds.current),
        () => !readOnlyRef.current && !conflictRef.current && canReplayReviewHistory.current())]),noteCommandHistoryPlugin(notesClient.history,()=>editor,()=>!readOnlyRef.current&&!conflictRef.current&&!isTextEditorHeld(document.pageId)&&!reviewLocked.current&&canReplayReviewHistory.current())],
    }))()],
    ...(initialContent.length === 0 ? {} : { initialContent }),
  }) as unknown as NoteEditorInstance;
  const historyRoot=useRef<HTMLElement|null>(null);
  useEffect(()=>{historyRoot.current=editor.domElement??null;},[editor]);
  useNoteHistoryScope(notesClient.history,historyRoot,!isReadOnly);
  useEffect(()=>{onEditorReady?.(editor);},[editor,onEditorReady]);
  // Focus after the actual retained editor mounts/becomes active, not while
  // its workspace is still rendering a loading placeholder.
  useEffect(()=>{if(autoFocus)editor.prosemirrorView.focus();},[editor,autoFocus]);
  useEffect(() => {
    const element = editor.domElement;
    if (!element) return;
    element.dataset.notePdfPageId = document.pageId;
    return () => { delete element.dataset.notePdfPageId; };
  }, [editor, document.pageId]);
  useEffect(() => {
    const port = createNoteEditorInstallPort({ editor, pageId: document.pageId,
      canInstall: () => !readOnlyRef.current && !composingRef.current && !assetImportRunningRef.current,
      canEdit: () => !readOnlyRef.current && !conflictRef.current && !reviewLocked.current,
      getTrustedDocument: () => trustedDocument.current,
      onInstalled: (installed) => { trustedDocument.current = installed; publishedDocument.current = JSON.stringify(getDocument()); },
    });
    installPortRef.current = port;
    const unregister = registerTextInstallPort(document.pageId, port, () => !composingRef.current && !assetImportRunningRef.current && !reviewLocked.current);
    onInstallPort?.(port);
    return () => {
      port.dispose();
      unregister();
      installPortRef.current = undefined;
      onInstallPort?.(undefined);
    };
  }, [editor, document.pageId, onInstallPort]);
  const validChildPages = (blocks: readonly NoteBlockV1[]): { blockId: string; targetId: string }[] => blocks.flatMap(block => [
    ...(block.type === "childPage" && block.content.kind === "reference" ? [{ blockId: block.id, targetId: block.content.targetId }] : []), ...validChildPages(block.children),
  ]);
  const getDocument = () => retainNoteProperties(blockNoteToNoteDocument(document.pageId, editor.document, { validChildPages: validChildPages(trustedDocument.current.blocks) }),trustedDocument.current,editor.schema.blockSchema);
  const useReview = session?.useReview ?? useNoteTextReview;
  const review = useReview({ editor, document, port: installPortRef, getDocument, onFormalChange: onChange });
  reviewLocked.current = review.locked;
  canReplayReviewHistory.current = () => !textHeld && review.canReplayHistory();
  trustedDocument.current = review.trustedDocument;
  reviewPendingIds.current = review.pendingBlockIds;
  // The review plugin observes PM updates before BlockEditor's composition
  // gate. Publish the final IME document through onChange at compositionend.
  reviewPublisher.current = () => { if (!composingRef.current && !installPortRef.current?.isInstalling) review.publishInput(); };
  // BlockNote also emits updates when editability changes. Compare the actual
  // editor projection so a save fence cannot turn a status update into input.
  const publishedDocument = useRef<string | undefined>(undefined);
  if (publishedDocument.current === undefined) publishedDocument.current = JSON.stringify(getDocument());
  const publishDocument = () => {
    const next = getDocument(), encoded = JSON.stringify(next);
    if (encoded === publishedDocument.current) return;
    publishedDocument.current = encoded;
    if (!installPortRef.current?.isInstalling && !review.publishInput()) onChange(next);
  };

  async function addManagedAssets(files: readonly File[], insertion: AssetInsertion): Promise<void> {
    const canInsert = () => alive.current && !readOnlyRef.current && !conflictRef.current
      && !heldRef.current && !isTextEditorHeld(document.pageId) && !reviewLocked.current;
    if (!canInsert()) return;
    if (assetBatchRunningRef.current) {notify.info("已有附件正在导入，请等待完成。"); return;}
    assetBatchRunningRef.current = true;
    try {
      let target = insertion;
      for (const file of files) {
        if (!canInsert()) return;
        let asset: Awaited<ReturnType<typeof importAsset>>;
        try {asset = await importAsset(file);} catch {return; /* Importer reports failure/cancellation. */}
        const block = editor.getBlock(target.blockId);
        if (!canInsert() || !block) {
          if (alive.current) notify.info("附件已导入；原插入位置已不可编辑，未插入正文。");
          return;
        }
        const value = {
          type: asset.mimeType.startsWith("image/") ? "managedImage" as const : "managedFile" as const,
          props: {assetId: asset.id, displayName: asset.displayName, mimeType: asset.mimeType, caption: "",
            ...(asset.mimeType.startsWith("image/") ? {previewScale: 100} : {})},
        };
        // Recheck the current block: typing during import must not be replaced.
        const empty = target.placement === "replaceEmpty" && block.type === "paragraph" && Array.isArray(block.content) && !block.content.length && !block.children.length;
        const inserted = empty ? editor.updateBlock(block, value)
          : editor.insertBlocks([value], block, target.placement === "before" ? "before" : "after")[0]!;
        target = {blockId: inserted.id, placement: "after"};
      }
    } catch (reason) {notify.error(reason instanceof Error ? reason.message : String(reason));}
    finally {assetBatchRunningRef.current = false;}
  }
  function addManagedAsset(file: File): Promise<void> {
    return addManagedAssets([file], {blockId: editor.getTextCursorPosition().block.id, placement: "after"});
  }

  async function cancelAssetImport(): Promise<void> {
    assetCancelRequestedRef.current = true;
    notify.info("正在取消附件导入…");
    try {
      await notesClient.gateway.cancelNoteAssetImport();
    } catch (reason: unknown) {
      notify.error(reason instanceof Error ? reason.message : String(reason));
    }
  }

  async function createLinkedPage(): Promise<void> {
    if (readOnlyRef.current || conflictRef.current || textHeld || review.locked) return;
    const blockId = editor.getTextCursorPosition().block.id;
    try {
      await flushNoteReview(document.pageId);
      const saved=await notesClient.gateway.loadDocument(document.pageId);
      const find=(blocks:readonly NoteBlockV1[]):NoteBlockV1|undefined=>{
        for(const block of blocks){if(block.id===blockId)return block;const child=find(block.children);if(child)return child;}return undefined;
      };
      const anchor=find(saved.document.blocks);
      if(!anchor)throw new Error("MODEL_DRAFT_SOURCE_CHANGED：原位置已变化，请重新选择新建页面的位置");
      const created = await notesClient.creation.create(false,{pageId:document.pageId,blockId,anchor});
      // The canonical page and source entry are committed before navigation.
      // A source write hold must not discard the newly created page.
      if (!alive.current) return;
      window.dispatchEvent(new CustomEvent("research-assistant:open-note-page",{detail:created.page.id}));
    } catch (reason) { reportNoteCreationError(reason); }
  }

  const slashItems = buildNoteDomainSlashItems({
    editor,
    onChoosePageLink: () => setPageLinkBlockId(editor.getTextCursorPosition().block.id),
    onChooseAsset: () => chooseAppFile(fileInputRef.current, file => void addManagedAsset(file), "asset"),
    onCreatePage: () => { void createLinkedPage(); },
  });
  const researchItems = buildResearchMentionItems(editor, [
    ...pageTargets.filter(page => page.id !== document.pageId).map(page => ({
      id: page.id, kind: "note_page" as const, label: page.title,
    })),
    ...readingTargets.map(reading => ({
      id: reading.id, kind: "reading_note" as const, label: reading.label, context: reading.quote,
    })),
    ...researchTargets,
  ]);

  return (
    <NoteEditorSurface
      blockSourcePath={["notes", document.pageId]}
      sourceNavigation={sourceNavigation?.pageId === document.pageId ? sourceNavigation : undefined}
      editor={editor} className={`note-editor${session?.surfaceClassName ? ` ${session.surfaceClassName}` : ""}`} readOnly={isReadOnly || conflictLocked || textHeld || review.locked}
      components={noteEditorComponents}
      onChange={publishDocument} movePolicy={noteBlockMovePolicy}
      onOpenSourceLink={openSourceLink}
      onCompositionChange={(composing) => { composingRef.current = composing; }}
      additionalSlashItems={slashItems} defaultFormattingToolbar={false} defaultLinkToolbar={false} ariaLabel="笔记正文"
      beforeEditor={<>{pageLinkBlockId && !isReadOnly && !conflictLocked && !textHeld && <NotePageLinkPicker
        pages={pageTargets.filter(page => page.id !== document.pageId)} onClose={() => { setPageLinkBlockId(undefined); editor.focus(); }}
        queryTargets={queryNoteTargets} excludePageId={document.pageId}
        onSelect={page => {
          if (!editor.getBlock(pageLinkBlockId) || readOnlyRef.current || conflictRef.current) return;
          editor.transact(transaction => { closeHistory(transaction); });
          editor.updateBlock(pageLinkBlockId, { type: "pageLink", props: { targetId: page.id, labelSnapshot: page.title } });
          editor.transact(transaction => { closeHistory(transaction); });
          setPageLinkBlockId(undefined); editor.focus();
        }} />}
        <NoteReviewRecoveryControl error={review.error} retry={review.request ? review.restore : undefined} target={diffControlsTarget} />{!isReadOnly && <>
        {session ? session.renderReview(editor,installPortRef) : <NoteEditorDiff editor={editor} chunks={review.chunks} disabled={review.disabled || textHeld} onResolve={review.resolve}
          {...(diffControlsTarget === undefined ? {} : { controlsTarget: diffControlsTarget })} />}
        {review.metadataPending && <div className="note-editor-associated-change" role="status">
          <span>{{ create: "新建页面及关联", move: "移动页面及关联", trash: "移入回收站", update: "页面标题或关联变化" }[review.metadataKind ?? "update"]}</span>
          <button disabled={review.disabled} onClick={() => review.resolve("undo")} type="button">Undo</button>
          <button disabled={review.disabled} onClick={() => review.resolve("keep")} type="button">Keep</button>
        </div>}

        <input hidden onChange={(event) => {
          const file = event.target.files?.[0];
          if (file !== undefined) void addManagedAsset(file);
          event.target.value = "";
        }} ref={fileInputRef} type="file" />
        {isImportingAsset && <button onClick={() => void cancelAssetImport()} type="button">取消附件导入</button>}
      </>}</>}
    >
        <NoteClipboard editor={editor} />
        <ManagedAssetIngress editor={editor} onFiles={(files, insertion) => {void addManagedAssets(files, insertion);}} />
        <LinkToolbarController linkToolbar={NoteLinkToolbar} />
        <NoteEvidenceMenu editor={editor} pageId={document.pageId}/>
        {!isReadOnly && !conflictLocked && <NoteSelectionAskAi editor={editor} pageId={document.pageId} />}
        {!isReadOnly && !conflictLocked && (researchItems.length > 0||queryNoteTargets) && <EditorSuggestionMenu
          suggestionMenuComponent={MentionSuggestionMenu}
          getItems={async (query) => {
            if(!queryNoteTargets)return filterNoteSuggestionItems(researchItems,query);
            try{
              const page=await queryNoteTargets(query);
              const targets=page.items.flatMap(item=>item.target.kind==="writing"||item.target.pageId===document.pageId?[]:[
                {id:item.target.pageId,kind:"note_page" as const,label:item.title},
                ...(item.target.kind==="reading"?[{id:item.target.noteId,kind:"reading_note" as const,label:item.owner?.label??item.title,context:item.snippet||item.title}]:[]),
              ]);
              return [...buildResearchMentionItems(editor,targets),...filterNoteSuggestionItems(buildResearchMentionItems(editor,researchTargets),query)];
            }catch(reason){notify.error(catalogErrorMessage(reason));return filterNoteSuggestionItems(researchItems,query);}
          }}
          triggerCharacter="@"
          floatingUIOptions={{ elementProps: { className: "note-mention-menu-popover" } }}
        />}
    </NoteEditorSurface>
  );
}
