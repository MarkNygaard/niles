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
// installed to a Home Screen, and, later, receiving a push when a timer
// fires. Push handlers land with that work; this is the shell they need
// to exist in first.

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
