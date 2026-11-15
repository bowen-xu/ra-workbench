import type { NoteAnnotationAccess, NoteResourceAccess, LiteratureNoteAccess, NoteResourceObservation } from "../../contracts/noteResources";
import type { NotesClient } from "./NotesClient";

const resources = new WeakMap<NotesClient, NoteResourceAccess>();
const annotations = new WeakMap<NotesClient, NoteAnnotationAccess>();
const literature = new WeakMap<NotesClient, LiteratureNoteAccess>();

function observation(client: NotesClient): NoteResourceObservation {
  return { subscribe: client.subscribe, getVersion: client.getVersion,
    getChangesSince: version => client.getChangesSince(version),
    notifyChanged: change => client.notifyChanged(change) };
}

/** Adapt the original provider once. This neither constructs a NotesClient nor
 * replaces its gateway, creation recovery, invalidation history or command owner. */
export function noteResourceAccess(client: NotesClient): NoteResourceAccess {
  let access = resources.get(client);
  if (access) return access;
  const source = client.gateway;
  const gateway: NoteResourceAccess["gateway"] = {
    ...(source.pageCatalog ? {pageCatalog: source.pageCatalog} : {}),
    listPages: (...args) => source.listPages(...args), listPageMetadata: (...args) => source.listPageMetadata(...args),
    listFavoritePageIds: (...args) => source.listFavoritePageIds(...args), reorderFavorites: (...args) => source.reorderFavorites(...args),
    applyMetadataBatch: (...args) => source.applyMetadataBatch(...args),
    ...(source.replayMetadataBatch ? {replayMetadataBatch: (...args) => source.replayMetadataBatch!(...args)} : {}),
    createPage: (...args) => source.createPage(...args), updatePage: (...args) => source.updatePage(...args),
    duplicatePage: (...args) => source.duplicatePage(...args), permanentlyDeletePage: (...args) => source.permanentlyDeletePage(...args),
    loadDocument: (...args) => source.loadDocument(...args), saveDocument: (...args) => source.saveDocument(...args),
    commitNavigation: (...args) => source.commitNavigation(...args), listRevisions: (...args) => source.listRevisions(...args),
    loadRevision: (...args) => source.loadRevision(...args), restoreRevision: (...args) => source.restoreRevision(...args),
    setRevisionPinned: (...args) => source.setRevisionPinned(...args), listOutgoing: (...args) => source.listOutgoing(...args),
    listReadingNotes: (...args) => source.listReadingNotes(...args), getReadingNote: (...args) => source.getReadingNote(...args),
    updateReadingNote: (...args) => source.updateReadingNote(...args), exportNotes: (...args) => source.exportNotes(...args),
    cancelNoteExport: (...args) => source.cancelNoteExport(...args),
  };
  access = { ...observation(client), gateway,
    links: { listOutgoing: client.links.listOutgoing, listBacklinks: client.links.listBacklinks,
      invalidate: change => client.links.invalidate(change) },
    creation: {create: (favorite, source) => client.creation.create(favorite, source),
      isDraft: id => client.creation.isDraft(id), forgetEmpty: id => client.creation.forgetEmpty(id)},
    history: {closeGroups: () => client.history.closeGroups(), record: command => client.history.record(command),
      run: (direction, id, target) => client.history.run(direction, id, target)},
    mutate: operation => client.mutate(() => operation(gateway)),
  };
  resources.set(client, access);
  return access;
}

export function noteAnnotationAccess(client: NotesClient): NoteAnnotationAccess {
  let access = annotations.get(client);
  if (!access) {
    const source = client.gateway;
    access = {...observation(client), gateway: {
      listReadingNotes: (...args) => source.listReadingNotes(...args), getReadingNote: (...args) => source.getReadingNote(...args),
      createReadingNote: (...args) => source.createReadingNote(...args), updateReadingNote: (...args) => source.updateReadingNote(...args),
      setReadingNoteStatus: (...args) => source.setReadingNoteStatus(...args),
    }};
    annotations.set(client, access);
  }
  return access;
}

export function literatureNoteAccess(client: NotesClient): LiteratureNoteAccess {
  let access = literature.get(client);
  if (!access) {
    access = {...observation(client), list: paperId => client.gateway.listLiteratureNotes(paperId),
      create: paperId => client.mutate(gateway => gateway.createLiteratureNote(paperId))};
    literature.set(client, access);
  }
  return access;
}
