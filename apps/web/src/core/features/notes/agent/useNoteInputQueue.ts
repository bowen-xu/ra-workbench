import {useEffect,useMemo,useRef,useState} from "react";
import type {AgentNoteQueueScope} from "@research-assistant/core/agent";
import {agentNoteQueueGateway,agentTurnGateway} from "../../../services/agentGateway";
import {NoteQueueSubmissionController,noteQueueSubmissionOwnerKey,noteQueueSubmissionOwners,type NoteQueueSubmissionSnapshot} from "../../agent/noteQueueSubmissionController";
import {useAgentInputQueue} from "../../agent/useAgentInputQueue";
import {useAgentQueueOwners} from "../../agent/AgentQueueProvider";

export function useNoteInputQueue(scope:AgentNoteQueueScope|null,ready:boolean,refresh:number) {
  const review=useRef<NoteQueueSubmissionController|null>(null);
  const [submission,setSubmission]=useState<NoteQueueSubmissionSnapshot|null>(null);
  const conversationId=scope?.conversationId;
  const kind=scope?.scope.kind;
  const resourceId=scope?.scope.resourceId;
  const owners=useAgentQueueOwners();
  const fixed=useMemo(() => scope ? { conversationId: scope.conversationId, scope: { kind: scope.scope.kind, resourceId: scope.scope.resourceId } } : null,
    [conversationId,kind,resourceId]);
  const {controller:edits,state,retainTransfer,acquireOwner}=useAgentInputQueue(agentNoteQueueGateway,fixed,ready,refresh,owners?.notes);
  useEffect(()=>{
    if(!fixed){ review.current=null; setSubmission(null); return; }
    const lease=noteQueueSubmissionOwners.acquire(noteQueueSubmissionOwnerKey(fixed),()=>new NoteQueueSubmissionController(fixed,agentNoteQueueGateway,agentTurnGateway));
    const submitted=lease.controller;
    review.current=submitted;
    setSubmission(submitted.getSnapshot());
    const unsubscribeReview=submitted.subscribe(()=>setSubmission(submitted.getSnapshot()));
    return()=>{unsubscribeReview();lease.release();review.current=null;};
  },[fixed]);
  return {edits,review,state,submission,retainTransfer,acquireOwner};
}
