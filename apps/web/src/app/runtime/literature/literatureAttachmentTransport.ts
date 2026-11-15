import {
  decodeReaderViewStateV2,
  upgradeLegacyReaderViewState,
  type ReaderViewStateV2,
} from "@research-assistant/core/reader";

export type AttachmentByteResponse = ArrayBuffer | Uint8Array | readonly number[];

export interface ReadingPositionResponse {
  readonly pageIndex: number;
  readonly zoom: number;
  readonly viewStateJson?: string | null;
}

/** Preserve the original binary IPC forms without a JSON/base64 transport. */
export function attachmentBytes(value: AttachmentByteResponse): Uint8Array {
  if (value instanceof Uint8Array) return value;
  if (value instanceof ArrayBuffer) return new Uint8Array(value);
  return Uint8Array.from(value);
}

export function readingStateArguments(viewState: ReaderViewStateV2) {
  return {
    pageIndex: viewState.pageIndex,
    zoom: viewState.scale.kind === "numeric" ? viewState.scale.value : 1,
    viewStateJson: JSON.stringify(viewState),
  };
}

/** Keep the original V2-first reading and legacy-column compatibility. */
export function readingState(value: ReadingPositionResponse | null): ReaderViewStateV2 | undefined {
  if (value === null) return undefined;
  if (value.viewStateJson !== undefined && value.viewStateJson !== null) {
    try {
      const decoded = decodeReaderViewStateV2(JSON.parse(value.viewStateJson));
      if (decoded !== undefined) return decoded;
    } catch {
      // Fall through to the compatible legacy upgrade below.
    }
  }
  return upgradeLegacyReaderViewState(value);
}
