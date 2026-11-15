import {openSystemDocument} from "../system-documents/openSystemDocument";
import {systemDocumentId} from "@research-assistant/core/agent";
import { X } from "lucide-react";
import { useEffect, useRef } from "react";
import { createPortal } from "react-dom";
import { agentFileSourceLink, type AgentReferenceInput, type AgentResourceScope } from "@research-assistant/core/agent";
import { navigateSource } from "./sourceNavigation";
import {useAgentExtensions} from "../../extensions/AgentExtensionProvider";
import { notify } from "../../services/notifications";
import type { AgentResourceGateways } from "./agentResourceGateways";
type AgentSkillToken = Extract<AgentReferenceInput, {type: "skill"}>;
import "./AgentReferenceDocument.css";
import { openTextDocument } from "../../components/text-editor/textDocumentEvents";

export function AgentReferenceDocument({reference, onClose, gateways, scope}: {
  readonly reference: AgentReferenceInput; readonly onClose: () => void;
  readonly gateways?: AgentResourceGateways; readonly scope?: AgentResourceScope;
}): React.JSX.Element {
  const builtinId = systemDocumentId(reference);
  if(builtinId) return <BuiltinReference id={builtinId} gateways={gateways} onClose={onClose}/>;
  if (reference.type === "skill") return <SkillDocument skill={reference} onClose={onClose}
    {...(gateways ? {gateways} : {})} {...(scope ? {scope} : {})}/>;
  if (reference.target.kind === "file") return <FileReference reference={reference} scope={scope} onClose={onClose}/>;
  return <ReferenceSource reference={reference} onClose={onClose}/>;
}
function BuiltinReference({id, gateways, onClose}: {readonly id: string; readonly gateways: AgentResourceGateways | undefined; readonly onClose: () => void}): null {
  const close = useRef(onClose); close.current = onClose;
  const gateway = gateways?.systemDocuments;
  useEffect(() => {
    let active = true;
    void openSystemDocument(id, gateway, () => active)
      .catch(error => {if (active) notify.error(error instanceof Error ? error.message : "找不到文件对象");})
      .finally(() => {if (active) close.current();});
    return () => {active = false;};
  }, [id, gateway]);
  return null;
}
function FileReference({reference, scope, onClose}: {readonly reference: Extract<AgentReferenceInput, {type: "mention"}>; readonly scope: AgentResourceScope | undefined; readonly onClose: () => void}): null {
  const close = useRef(onClose); close.current = onClose;
  useEffect(() => {
    let active = true;
    void Promise.resolve().then(() => {
      if (reference.target.kind !== "file") return;
      const href = agentFileSourceLink(reference, scope);
      if (!href) throw new Error("找不到文件对象");
      return navigateSource(href);
    }).catch(() => {if (active) notify.error("找不到文件对象");})
      .finally(() => {if (active) close.current();});
    return () => {active = false;};
  }, [reference, scope]);
  return null;
}
function ReferenceSource({reference, onClose}: {readonly reference: Extract<AgentReferenceInput, {type: "mention"}>; readonly onClose: () => void}): React.JSX.Element {
  const dialog = useRef<HTMLDialogElement>(null);
  useEffect(() => {const element = dialog.current; element?.showModal(); return () => element?.close();}, []);
  function close() {dialog.current?.close(); onClose();}
  const target = reference.target;
  return createPortal(<dialog ref={dialog} className="agent-skill-document" aria-label={`引用来源 ${reference.name}`}
    onCancel={event => {event.preventDefault(); close();}}>
    <header><h2>{reference.name}</h2><button type="button" aria-label="关闭引用来源" onClick={close}><X aria-hidden="true" size={18}/></button></header>
    <p className="agent-skill-document-source">{target.kind === "tool" ? `${target.environmentId} · ${target.serverId}` : `${target.resourceId} · ${target.sourceId}`}<br/>{target.revision}</p>
  </dialog>, document.body);
}

function SkillDocument({skill, onClose, gateways, scope}: {
  readonly skill: AgentSkillToken; readonly onClose: () => void;
  readonly gateways?: AgentResourceGateways; readonly scope?: AgentResourceScope;
}): null {
  const close = useRef(onClose); close.current = onClose;
  const extensions = useAgentExtensions();
  const read = gateways ? gateways.readSkillDocument : extensions.skills?.readDocument;
  useEffect(() => {
    let active = true;
    const result = Promise.resolve().then(() => {
      if (!read) throw new Error("Unavailable resource gateway");
      return read(skill.identity, scope);
    }).catch(() => {throw new Error("找不到文件对象");});
    void result.then(content => {
      if (!active) return;
      if (!openTextDocument({id: `skill:${skill.identity.environment_id}:${skill.identity.source_id}`,
        reopen:{kind:"skill",identity:skill.identity,...(scope ? {scope} : {})},
        path: skill.identity.source_id, title: skill.name, content, readOnly: true})) throw new Error("无法打开文本标签页。");
    }).catch(error => {if (active) notify.error(error instanceof Error ? error.message : "找不到文件对象");})
      .finally(() => {if (active) close.current();});
    return () => {active = false;};
  }, [read, scope, skill]);
  return null;
}
