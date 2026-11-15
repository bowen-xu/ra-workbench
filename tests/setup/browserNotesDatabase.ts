import {beforeEach} from "vitest";
import {configureBrowserNotesDatabase} from "../../apps/web/src/core/features/notes/gateways/browserNotesDatabase";
import {createMemoryNotesDatabase} from "../../apps/web/src/core/runtime/testing/memoryNotesDatabase";
import {configurePreviewDatabase} from "../../apps/web/src/core/runtime/previewIndexedDatabase";
import {createMemoryPreviewDatabase} from "../../apps/web/src/app/runtime/testing/memoryPreviewDatabase";
configureBrowserNotesDatabase(createMemoryNotesDatabase().open);
beforeEach(() => configurePreviewDatabase(createMemoryPreviewDatabase().open));
