// SPDX-License-Identifier: AGPL-3.0-or-later
interface UpstreamConnector {
  isOnline: boolean;
  clientVersion: string;
  prefs: Record<string, boolean>;
  callMethod: ReturnType<typeof ResearchAssistant.createTransport>;
  checkIsOnline: () => Promise<boolean>;
  reportActiveURL: () => void;
  CommunicationError: typeof ResearchAssistant.CommunicationError;
  ping: (payload?: unknown) => Promise<unknown>;
  _processPreferences: (value: Record<string, boolean>) => void;
}
declare const Zotero: {
  version: string;
  Connector: UpstreamConnector;
  Connector_Debug: { submitReport: () => Promise<never> };
  Repo: {
    getTranslatorCode: (id: string) => Promise<string>;
    getTranslatorMetadataFromZotero: () => Promise<unknown>;
    getTranslatorMetadataFromServer: () => Promise<unknown>;
  };
  Translators: { keepTranslatorsUpdated: () => Promise<void> };
  Prefs: { syncStorage: Record<string, unknown>; init: () => Promise<void> };
  ContentTypeHandler: { init: () => void; enable: () => void; disable: () => void };
  API: unknown;
  GoogleDocs: unknown;
  GoogleDocs_API: unknown;
  Inject: {
    checkActionToServer: () => Promise<boolean>;
    confirm: (props: Record<string, string>) => Promise<unknown>;
  };
};
