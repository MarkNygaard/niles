// Niles's service worker.
//
// It caches nothing, on purpose. This page exists to say which lights
// are on; a cached copy is a page confidently describing a room you are
// standing in and can see is wrong. There is also nothing worth having
// offline — every control here needs Niles on the other end of it — so
// the usual trade of "stale but present" buys nothing and costs
// correctness.
//
// What it is for is the two things a page cannot do on its own: being
// installed to a Home Screen, and receiving a push while the app is
// closed: "Your Thailand travel plan is ready", and later a timer that
// fires. The pushes are sent by crates/niles-webpush.

// Take over as soon as a new version is published, rather than waiting
// for every tab to close. A service worker this small has no state to
// hand over, so the usual reason to wait doesn't apply.
self.addEventListener("install", () => self.skipWaiting());
self.addEventListener("activate", (event) => event.waitUntil(self.clients.claim()));

// Deliberately empty, and deliberately present.
//
// Chrome will not offer to install a page whose service worker has no
// fetch handler. Not calling `respondWith` leaves the request to the
// network exactly as if this listener were absent — which is what we
// want — so the listener is the whole of it. Removing it as dead code
// would silently cost installability on Android.
self.addEventListener("fetch", () => {});

// A push is always shown, whatever it holds. Subscribing promised the
// browser as much (`userVisibleOnly`), and Safari takes the
// subscription away from a worker that receives a push and shows
// nothing.
self.addEventListener("push", (event) => {
  let message = {};
  try {
    message = event.data ? event.data.json() : {};
  } catch {
    message = { body: event.data ? event.data.text() : "" };
  }
  const tag = typeof message.tag === "string" && message.tag ? message.tag : undefined;
  event.waitUntil(
    self.registration.showNotification(message.title || "Niles", {
      body: message.body || "",
      icon: "/icon-192.png",
      badge: "/icon-192.png",
      tag,
      // A ready after a failure under the same tag replaces it, and
      // should still be heard rather than swapped in silently.
      renotify: Boolean(tag),
      data: { url: message.url || "/" },
    }),
  );
});

// A tap: Niles open somewhere is brought forward and sent to the
// notification's address — the page follows it (src/lib/pwa.ts), which
// a hash change does without reloading. With none open, one opens there.
self.addEventListener("notificationclick", (event) => {
  event.notification.close();
  const url = new URL(event.notification.data?.url || "/", self.location.origin).href;
  event.waitUntil(
    (async () => {
      const windows = await self.clients.matchAll({ type: "window", includeUncontrolled: true });
      const open = windows.find((client) => new URL(client.url).origin === self.location.origin);
      if (open) {
        await open.focus();
        open.postMessage({ type: "open", url });
        return;
      }
      await self.clients.openWindow(url);
    })(),
  );
});
