import type { NotificationList } from "@/lib/api";

/** Everything about the signed-in person's notifications. A frame on
    the stream marks all of it stale. */
export const NOTIFICATIONS = ["notifications"];
/** The newest page, which is where the bell reads its count. */
export const LATEST = [...NOTIFICATIONS, "latest"];
/** Every page the Notifications page has shown. */
export const PAGES = [...NOTIFICATIONS, "pages"];

/** What the bell says: nothing at all with nothing unread. */
export function bellCount(list: NotificationList | undefined): number {
  return list?.unread ?? 0;
}

/** The newest one listed, which "read up to" marks to. */
export function newestId(list: NotificationList | undefined): number | undefined {
  return list?.notifications[0]?.id;
}

/** A link as the app follows it: `/#/chat?artifact=7` is `#/chat?artifact=7`. */
export function hrefOf(url: string): string {
  return url.startsWith("/#") ? url.slice(1) : url;
}
