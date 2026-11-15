/** Shared notification boundary; retains safe actionable diagnostics. */
export { notify } from "../services/notifications";
export type { NotificationKind, NotificationOptions, NotificationUpdate, NotificationAction } from "../services/notifications";
export { useNotificationState } from "../services/useNotificationState";
export { NotificationEffect } from "../components/NotificationEffect";
