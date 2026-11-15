import { useEffect, useState } from "react";
import { ChevronRight, FileDown } from "lucide-react";
import { ActionMenu, ActionMenuItem } from "../../../components/ActionMenu";
import { getAppRuntime } from "../../../runtime/appRuntime";
import { invoke } from "../../../runtime/platformCore";
import { useNotesClient } from "../NotesClient";
import { startNoteDocumentExport } from "./noteDocumentExport";
export function NoteExportMenu({ pageId, title }: {
    readonly pageId: string;
    readonly title: string;
}): React.JSX.Element {
    const client = useNotesClient();
    const [pdfAvailable, setPdfAvailable] = useState(false);
    useEffect(() => {
        if (getAppRuntime().mode !== "desktop")
            return;
        let active = true;
        void invoke<{
            pdf: boolean;
        }>("note_document_export_capabilities").then(result => { if (active)
            setPdfAvailable(result.pdf); }, () => { if (active)
            setPdfAvailable(false); });
        return () => { active = false; };
    }, []);
    return <ActionMenu label="导出" className="note-export-menu" contentClassName="note-page-action-menu" showTitle={false} portal triggerRender={<button type="button"/>} trigger={<><FileDown aria-hidden="true" size={16}/><span>导出</span><ChevronRight aria-hidden="true" size={16}/></>}>
    <ActionMenuItem disabled={!pdfAvailable} aria-label={pdfAvailable ? "导出 PDF" : "导出 PDF（仅支持原生 PDF 的桌面应用可用）"} onClick={() => void startNoteDocumentExport("pdf", pageId, title, id => client.gateway.readNoteAsset(id))}>导出 PDF</ActionMenuItem>
    <ActionMenuItem onClick={() => void startNoteDocumentExport("html", pageId, title, id => client.gateway.readNoteAsset(id))}>导出 HTML</ActionMenuItem>
  </ActionMenu>;
}
