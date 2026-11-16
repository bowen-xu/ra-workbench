import { expect, it, vi } from "vitest";
import { submitSelectionAnnotation } from "../../../../../../../../plugins/ra-literature-library/src/features/reader/selection/submitSelectionAnnotation";
import { releaseRemovedEvidence } from "../../../../../../../apps/web/src/core/features/agent/evidenceImageGateway";
vi.mock("../../../../../../../apps/web/src/core/features/agent/evidenceImageGateway",()=>({releaseRemovedEvidence:vi.fn(),captureEvidenceImage:vi.fn()}));
const pending={text:"原文",rects:[],prefix:"",suffix:"",pageIndex:0,pageLabel:"1",segments:[],popupPageIndex:0,popupRect:{x:0,y:0,width:.1,height:.1}};
it.each(["","可选评论"])("silently persists the quotation with comment %s without opening or sending, then ends the temporary selection",async comment=>{
  const submit=vi.fn(async()=>undefined),discard=vi.fn();
  await submitSelectionAnnotation({paperId:"paper",attachmentId:"pdf",title:"paper.pdf",pending,controller:{renderPageRegionPng:vi.fn()},comment,intent:"silent",submit,discard});
  expect(submit).toHaveBeenCalledExactlyOnceWith("",expect.any(Array),{mode:"draft",reveal:false,evidence:[expect.objectContaining({text:"原文",comment})]});
  expect(discard).toHaveBeenCalledOnce();
});
it("adds a draft or submits the comment as a question with exact evidence, without screenshotting selected text",async()=>{
  const submit=vi.fn(async()=>undefined),discard=vi.fn(),renderPageRegionPng=vi.fn();
  const options={paperId:"paper",attachmentId:"pdf",title:"paper.pdf",pending,controller:{renderPageRegionPng},comment:"为什么？",submit,discard};
  await submitSelectionAnnotation(options);
  expect(submit).toHaveBeenLastCalledWith("",[{kind:"paper",resourceId:"paper",attachmentId:"pdf"}],expect.objectContaining({mode:"draft",evidence:[expect.objectContaining({source:"source://library/paper/pdf",text:"原文",comment:"为什么？"})]}));
  await submitSelectionAnnotation({...options,intent:"send"});
  expect(submit).toHaveBeenLastCalledWith("为什么？",expect.any(Array),{evidence:[expect.objectContaining({source:"source://library/paper/pdf",text:"原文",comment:"为什么？"})]});
  expect(discard).toHaveBeenCalledTimes(2);expect(renderPageRegionPng).not.toHaveBeenCalled();
});
it("retains a failed comment/selection and releases unattached captures without dispatching an empty question",async()=>{
  const submit=vi.fn().mockRejectedValue(new Error("忙")),discard=vi.fn(),renderPageRegionPng=vi.fn();
  const options={paperId:"paper",attachmentId:"pdf",title:"paper.pdf",pending,controller:{renderPageRegionPng},comment:"问题",submit,discard,intent:"send" as const};
  await expect(submitSelectionAnnotation(options)).rejects.toThrow("忙");expect(discard).not.toHaveBeenCalled();expect(releaseRemovedEvidence).toHaveBeenCalled();
  submit.mockClear();await expect(submitSelectionAnnotation({...options,comment:" "})).rejects.toThrow("请输入");expect(submit).not.toHaveBeenCalled();
});
