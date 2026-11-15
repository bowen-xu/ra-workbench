import { useEffect, useRef, useState } from "react";
import { getAppStorage } from "../../runtime/appStorage";
import { acquireConversationRenameOwner, type ConversationRenameController, type ConversationRenamePorts, type ConversationRenameSnapshot } from "./conversationRenameController";

/** View subscriptions never own or cancel an admitted catalog operation. */
export function useConversationRename(id: string, ports: ConversationRenamePorts) {
  const storage = getAppStorage(), owner = useRef<ConversationRenameController | null>(null);
  const key = id.toLowerCase();
  const [owned, setOwned] = useState<{ key: string; state: ConversationRenameSnapshot } | null>(null);
  useEffect(() => {
    const lease = acquireConversationRenameOwner(id, ports, storage);
    const controller = lease.controller;
    owner.current = controller;
    const sync = () => setOwned({ key, state: controller.getSnapshot() });
    const detach = controller.subscribe(sync); sync();
    return () => { detach(); lease.release(); if (owner.current === controller) owner.current = null; };
  }, [key, ports.catalog, ports.metadata, storage]);
  async function retained<T>(operation: (value: ConversationRenameController) => Promise<T>): Promise<T> {
    const lease = acquireConversationRenameOwner(id, ports, storage);
    try { return await operation(lease.controller); } finally { lease.release(); }
  }
  return { state: owned?.key === key ? owned.state : null,
    getSnapshot: () => owner.current?.getSnapshot() ?? null,
    submit: (expectedTitle: string, title: string) => retained(value => value.submit(expectedTitle, title)),
    check: () => retained(value => value.check()),
    retryOriginal: () => retained(value => value.retryOriginal()),
    readCurrent: () => retained(value => value.readCurrent()),
    dismiss: () => owner.current?.dismiss() ?? false,
  };
}
