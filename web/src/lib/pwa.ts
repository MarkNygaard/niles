/**
 * Installing Niles to a Home Screen.
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

export function registerServiceWorker(): void {
  if (!("serviceWorker" in navigator)) return;

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
