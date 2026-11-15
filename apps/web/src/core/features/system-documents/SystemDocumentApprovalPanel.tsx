import { useEffect, useState } from "react";
import { ShieldCheck } from "lucide-react";
import type { SystemDocumentApproval, SystemDocumentGateway } from "@research-assistant/core/agent";
import { systemDocumentGateway } from "../../services/systemDocumentGateway";
import { notify } from "../../services/notifications";
import { TextDiffEditor } from "../../components/editor-diff/TextDiffEditor";
import "./SystemDocumentApprovalPanel.css";
/** A decision authorizes exactly this proposed write, never the file or session. */
export function SystemDocumentApprovalPanel({ conversationId, gateway = systemDocumentGateway }: {
    readonly conversationId: string;
    readonly gateway?: SystemDocumentGateway;
}) {
    const [requests, setRequests] = useState<readonly SystemDocumentApproval[]>([]);
    const [busy, setBusy] = useState(false);
    useEffect(() => {
        let active = true, inFlight = false;
        setRequests([]);
        const refresh = async () => {
            if (inFlight)
                return;
            inFlight = true;
            try {
                const next = await gateway.pending(conversationId);
                if (active)
                    setRequests(next);
            }
            catch { /* Non-native hosts have no pending write capability. Never grant on error. */ }
            finally {
                inFlight = false;
            }
        };
        void refresh();
        const timer = setInterval(() => void refresh(), 750);
        return () => { active = false; clearInterval(timer); };
    }, [conversationId, gateway]);
    async function decide(request: SystemDocumentApproval, allow: boolean) {
        setBusy(true);
        try {
            await gateway.decide(conversationId, request.id, allow);
            setRequests(rows => rows.filter(row => row.id !== request.id));
        }
        catch (reason) {
            notify.error(String(reason));
        }
        finally {
            setBusy(false);
        }
    }
    if (!requests.length)
        return null;
    return <section className="system-document-approval" aria-label="内置文档写入授权">{requests.map(request => <div key={request.id}>
    <header><ShieldCheck size={17}/><strong>允许修改内置文档？</strong></header>
    <p>{request.name}</p><small>{request.path}</small>
    <details><summary>查看本次修改</summary><TextDiffEditor revealFirstChange content={request.after} baseline={request.before} label="待授权的修改"/></details>
    <footer><button type="button" disabled={busy} onClick={() => void decide(request, false)}>拒绝</button><button type="button" disabled={busy} onClick={() => void decide(request, true)}>允许本次写入</button></footer>
  </div>)}</section>;
}
