import { useSyncExternalStore } from "react";

/**
 * Where the app is, read off the hash: `#/me/settings` is `/me/settings`.
 *
 * The hash rather than the path because the API owns the paths. It sits
 * at the root — `/me`, `/rooms`, `/climate` — and wins over the UI's
 * fallback, so a page at `/me` would come back as JSON on a refresh.
 * Nothing after a `#` reaches the server, so no page name can ever
 * collide with an endpoint, now or when one is added.
 */
export function routeOf(hash: string): string {
  const path = hash.replace(/^#/, "").split("?")[0];
  if (!path.startsWith("/")) return "/";
  return path.length > 1 ? path.replace(/\/+$/, "") : path;
}

/**
 * What a link asks of the page it opens, after a `?` in the hash:
 * `#/chat?artifact=12` is the chat with artifact 12 open. In the hash
 * for the same reason the page is, and so a notification can carry it.
 */
export function hashQuery(hash: string): URLSearchParams {
  const at = hash.indexOf("?");
  return new URLSearchParams(at < 0 ? "" : hash.slice(at + 1));
}

function subscribe(onChange: () => void): () => void {
  window.addEventListener("hashchange", onChange);
  return () => window.removeEventListener("hashchange", onChange);
}

export function useRoute(): string {
  return useSyncExternalStore(subscribe, () => routeOf(window.location.hash));
}
