import {invoke as invokePlatform} from "../../../core/runtime/platformCore";
import {decodeWritingCheckpointRecord, decodeWritingDocumentCheckpoint} from "@research-assistant/writing";
import type {WritingCheckpointGateway} from "@research-assistant/writing-projects-plugin/contracts/writingCheckpointGateway";

export function createTauriWritingCheckpointGateway(invoke: typeof invokePlatform = invokePlatform): WritingCheckpointGateway {
  return {
    async save(write) {
      const expected = decodeWritingDocumentCheckpoint(write.checkpoint);
      const saved = decodeWritingCheckpointRecord(await invoke<unknown>("save_writing_document_checkpoint", { write }));
      if (JSON.stringify(saved.checkpoint) !== JSON.stringify(expected)) throw new Error("写作检查点回执与请求不匹配");
      return saved;
    },
    async list(manuscriptId) {
      const rows = await invoke<unknown>("list_writing_document_checkpoints", { manuscriptId });
      if (!Array.isArray(rows)) throw new Error("无效的写作检查点响应");
      return rows.map((value) => {
        const row = decodeWritingCheckpointRecord(value);
        if (row.checkpoint.manuscriptId !== manuscriptId) throw new Error("写作检查点稿件身份不匹配");
        return row;
      });
    },
  };
}

export const tauriWritingCheckpointGateway = createTauriWritingCheckpointGateway();
