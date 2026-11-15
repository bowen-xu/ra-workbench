import {decodeWritingNoteCatalogPage,decodeWritingNoteCatalogPreparation,NotesGatewayError,type NotesErrorCode,type WritingNoteCatalogGateway} from "@research-assistant/data/notes";
import {invoke as invokePlatform} from "../../../core/runtime/platformCore";

async function command(invoke: typeof invokePlatform, name: string, args: Record<string,unknown>): Promise<unknown> {
  try {return await invoke(name,args);}
  catch (reason: unknown) {
    if (reason instanceof NotesGatewayError) throw reason;
    if (reason && typeof reason === "object") {
      const error = reason as {code?: unknown;message?: unknown;currentRevision?: unknown};
      throw new NotesGatewayError(typeof error.code === "string" ? error.code as NotesErrorCode : "operation_unknown",
        typeof error.message === "string" ? error.message : reason instanceof Error ? reason.message : String(reason),
        Number.isSafeInteger(error.currentRevision) && (error.currentRevision as number) >= 0 ? error.currentRevision as number : undefined);
    }
    throw new NotesGatewayError("operation_unknown",String(reason));
  }
}

export function createTauriWritingNoteCatalogGateway(invoke: typeof invokePlatform = invokePlatform): WritingNoteCatalogGateway {
  return {
    async queryWritingNoteCatalog(query) {
      return decodeWritingNoteCatalogPage(await command(invoke, "query_writing_note_catalog",{query}),query.limit ?? 20);
    },
    async prepareWritingNoteCatalog(options) {
      return decodeWritingNoteCatalogPreparation(await command(invoke, "prepare_writing_note_catalog",{retryErrors: options?.retryErrors ?? false,...(options?.scope ? {scope:options.scope} : {})}));
    },
  };
}

export const tauriWritingNoteCatalogGateway = createTauriWritingNoteCatalogGateway();
