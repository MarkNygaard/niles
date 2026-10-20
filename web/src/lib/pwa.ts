/**
 * Installing Niles to a Home Screen, and taking a notification about an
 * answered approval off this device once Niles's own push has had its
 * chance to replace it.
 *
 * There is no install button here and there should not be. Every
 * browser already has one — Chrome's own prompt, Safari's Share → Add
 * to Home Screen — and a button in the page can only reproduce it
 * badly: it cannot trigger Safari's, and it has to guess at whether
 * Chrome's is available.
 *
 * So the whole of the work is registering the service worker, which is
 * what makes the browser's own offer possible.
 */

/** Where the worker lives, and therefore its scope: the whole app. */
const WORKER = "/sw.js";

/** How long Niles's push replacing an answered approval's notification
    gets to arrive before the open app takes the waiting one off itself.
    Replaced, it changes quietly. Taken off first, the replacement would
    arrive as a new notification. */
export const REPLACED_WITHIN_MS = 10_000;

/** Takes off this device the notifications tagged `tag` that still wait
    for an answer, the ones with buttons. One saying what became of it
    stays. */
export async function closeWaiting(tag: string): Promise<void> {
  try {
    if (!("serviceWorker" in navigator)) return;
    const registration = await navigator.serviceWorker.getRegistration();
    for (const shown of (await registration?.getNotifications({ tag })) ?? []) {
      if (shown.data?.approval) shown.close();
    }
  } catch {
    // No worker here, or no notifications to look at: nothing waits.
  }
}

/** The same, once Niles's own push has had its chance. */
export function closeWaitingLater(tag: string): void {
  setTimeout(() => void closeWaiting(tag), REPLACED_WITHIN_MS);
}

export function registerServiceWorker(): void {
  if (!("serviceWorker" in navigator)) return;

  // A tapped notification, when Niles was already open: the worker
  // focuses this window and says where to go. Only this app's own
  // addresses are followed.
  navigator.serviceWorker.addEventListener("message", (event: MessageEvent) => {
    const url: unknown = event.data?.type === "open" ? event.data.url : undefined;
    if (typeof url !== "string") return;
    const target = new URL(url, window.location.origin);
    if (target.origin === window.location.origin) window.location.assign(target.href);
  });

  // After load, not during it. Registration competes with the first
  // render for the same connection, and nothing on the page is waiting
  // on the worker.
  window.addEventListener("load", () => {
    navigator.serviceWorker.register(WORKER).catch((error) => {
      // Not fatal: the page works, it just can't be installed. Worth a
      // line in the console, because "install isn't offered" is
      // otherwise a silent absence with no clue attached.
      console.warn("Niles could not register its service worker:", error);
    });
  });
}
