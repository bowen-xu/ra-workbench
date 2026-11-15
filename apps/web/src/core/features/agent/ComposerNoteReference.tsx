import {FileText} from "lucide-react";
import {useEffect, useLayoutEffect, useState} from "react";
import {createRoot, type Root} from "react-dom/client";
import {WidgetType, type EditorView} from "@codemirror/view";
import {decodeAgentContextResource, type AgentReferenceInput} from "@research-assistant/core/agent";
import {notePageId} from "@research-assistant/core/notes";
import {ControlHint} from "../../components/ControlHint";
import {InlineReference} from "../../components/InlineReference";
import {getLiteratureGateway} from "../../services/sourceDomainGateways";
import {getNotesGateway} from "../notes/gateways";
import {sourceLinkHint} from "./sourceLinkHint";

/** Display metadata never changes the bound source identity or its permissions. */
export function composerNoteId(input: AgentReferenceInput): string | undefined {
  if (input.type !== "mention" || input.target.kind !== "file" || !input.target.sourceId.startsWith("context:")) return undefined;
  try {
    const resource = decodeAgentContextResource(JSON.parse(input.target.sourceId.slice(8)));
    return resource.kind === "note" ? resource.resourceId : undefined;
  } catch {return undefined;}
}

function ComposerNoteReference({input, id, open, measure}: {readonly input: AgentReferenceInput; readonly id: string; readonly open: (input: AgentReferenceInput) => void; readonly measure: () => void}) {
  const [presentation, setPresentation] = useState({title:input.name, lines:[input.name] as readonly string[]});
  useLayoutEffect(measure, [presentation.title,measure]);
  useEffect(() => {
    let active = true;
    const notes = getNotesGateway(), pageId = notePageId(id);
    void notes.loadDocument(pageId).then(async snapshot => {
      const title = snapshot.page.title || "未命名笔记";
      if (active) setPresentation({title,lines:[title]});
      const links = await notes.listOutgoing(pageId);
      const owners = links.filter(link => link.relation === "derived_from" && link.target.kind === "paper" && !link.isBroken);
      const sources = await Promise.all(owners.map(async owner => sourceLinkHint({kind:"paper",item:await getLiteratureGateway().get(owner.target.id)})[0]!));
      if (active) setPresentation({title,lines:[title,...sources]});
    }).catch(() => { /* A missing source must not remove the draft or its binding. */ });
    return () => {active=false;};
  }, [id]);
  return <ControlHint content={<span className="control-hint-lines">{presentation.lines.map((line,index)=><span key={index}>{line}</span>)}</span>}>
    <InlineReference as="a" label={presentation.title} title=""
      href={`agent-resource:${encodeURIComponent(JSON.stringify(input))}`} tabIndex={-1} contentEditable={false}
      aria-label={`文件 ${presentation.title}`} icon={<FileText aria-hidden="true"/>} className="agent-composer-reference"
      onMouseDown={event=>event.preventDefault()} onClick={event=>{event.preventDefault();open(input);}}/>
  </ControlHint>;
}

type ReferenceDOM = HTMLSpanElement & {referenceRoot?:Root};
/** Uses the shared label and hint components inside CodeMirror's atomic range. */
export class ComposerNoteReferenceWidget extends WidgetType {
  constructor(readonly input: AgentReferenceInput, readonly id: string, readonly open: (input: AgentReferenceInput)=>void) {super();}
  override eq(other: ComposerNoteReferenceWidget): boolean {return this.open===other.open && JSON.stringify(this.input)===JSON.stringify(other.input);}
  toDOM(view: EditorView): HTMLElement {
    const element:ReferenceDOM=document.createElement("span");element.contentEditable="false";
    const root=createRoot(element);element.referenceRoot=root;
    root.render(<ComposerNoteReference input={this.input} id={this.id} open={this.open} measure={()=>{if(view.dom.isConnected)view.requestMeasure();}}/>);
    return element;
  }
  override destroy(dom: HTMLElement): void {
    const element=dom as ReferenceDOM, root=element.referenceRoot;
    if(root){delete element.referenceRoot;queueMicrotask(()=>root.unmount());}
  }
  override ignoreEvent(): boolean {return true;}
}
