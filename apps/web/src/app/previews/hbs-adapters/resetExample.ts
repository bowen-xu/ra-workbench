import type { DocumentPort, HbsDocumentAdapter, HbsSession } from "@research-assistant/core/hbs";
import type { HbsDeliveryPort } from "../../../core/components/hbs/deliveryPort";

/** Reset is a harness lifecycle boundary, never a review/Keep operation. */
export async function resetExample<D>(options: {
  readonly session: Pick<HbsSession<D>, "dispose" | "flushPreservation">;
  readonly delivery: Pick<HbsDeliveryPort<D>, "dispose" | "settle">;
  readonly actual: DocumentPort<D>;
  readonly adapter: HbsDocumentAdapter<D>;
  readonly fixture: D;
  readonly external: readonly Promise<void>[];
  readonly clearPreservation: () => Promise<void>;
  readonly readPreservation: () => Promise<unknown>;
}): Promise<void> {
  options.session.dispose();
  options.delivery.dispose();
  await Promise.all([options.delivery.settle(), options.session.flushPreservation(), ...options.external]);
  const current = await options.actual.read();
  if (!options.adapter.equal(current.saved, options.fixture) || !options.adapter.equal(current.baseline, options.fixture)) {
    await options.actual.commit({ id: crypto.randomUUID(), expectedVersion: current.version, saved: options.fixture, baseline: options.fixture });
  }
  await options.clearPreservation();
  const [saved, preservation] = await Promise.all([options.actual.read(), options.readPreservation()]);
  if (!options.adapter.equal(saved.saved, options.fixture) || !options.adapter.equal(saved.baseline, options.fixture) || preservation !== null) {
    throw new Error("MODEL_RESET_VERIFY_FAILED：重置后的 S/B 或编辑保全仍不是初始状态，请重试重置。");
  }
}
