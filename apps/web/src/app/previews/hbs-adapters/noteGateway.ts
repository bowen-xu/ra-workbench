import { NotesGatewayError, type NotesGateway } from "@research-assistant/data/notes";
import { previewCall } from "./storage";
/** Production renderer with synthetic assets; missing sources stay unavailable. */
export const noteRendererGateway = new Proxy({
  readNoteAsset: async (id: string) => {
    const value = await previewCall("note","readAsset",id);
    if(!Array.isArray(value)||value.some(byte=>!Number.isInteger(byte)||byte<0||byte>255))throw new Error("MODEL_PROTOCOL_ERROR：附件回执无效。");
    return value as number[];
  },
  getReadingNote: async () => { throw new NotesGatewayError("not_found", "此合成预览没有对应的阅读来源。"); },
}, {get(target,key) {if(key in target)return target[key as keyof typeof target];
  return async()=>{throw new Error(`MODEL_PREVIEW_COMMAND_UNSUPPORTED：该独立 HBS 预览没有启用 ${String(key)}。`);};}}) as unknown as NotesGateway;
