import {invoke} from "../../../core/runtime/platformCore";
import {decodeCollectionState} from "@research-assistant/core/collections";
import {decodeNalState} from "@research-assistant/core/nal";
import type {CollectionGateway} from "@research-assistant/data/collections";
import type {NalGateway, NalQueryGateway} from "@research-assistant/data/nal";
import {collectionCommandError, decodeNalAnnotationResponse, decodeNalQueryStore} from "@research-assistant/literature-library-plugin/transportDecoders";

export const tauriCollectionGateway: CollectionGateway = {
  async read() {
    try {return decodeCollectionState(await invoke("read_collection_network"));}
    catch (reason) {throw collectionCommandError(reason);}
  },
  async mutate(mutation, expectedRevision) {
    try {return decodeCollectionState(await invoke("mutate_collection_network", {mutation, expectedRevision}));}
    catch (reason) {throw collectionCommandError(reason);}
  },
};
export const tauriNalGateway: NalGateway = {
  async read() {return decodeNalState(await invoke("read_nal_network"));},
  async mutate(mutation, expectedRevision) {return decodeNalState(await invoke("mutate_nal_network", {mutation, expectedRevision}));},
  async ensureAnnotation(request) {return decodeNalAnnotationResponse(await invoke("ensure_nal_annotation", {request}), request);},
};
export const tauriNalQueryGateway: NalQueryGateway = {
  async read() {return decodeNalQueryStore(await invoke("read_nal_query_collections"));},
  async save(value, expectedRevision) {return decodeNalQueryStore(await invoke("save_nal_query_collections", {value, expectedRevision}));},
};
