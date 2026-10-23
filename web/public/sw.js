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
// closed: "Your Thailand travel plan is ready", or something waiting
// for a yes, which Android and computers can answer from the
// notification itself. The pushes are sent by crates/niles-webpush.

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

const ICON = "/icon-192.png";

// An approval's buttons, by what each does. Which a notification has is
// Niles's to say (crates/niles-bin/src/approvals/push.rs): Deny always,
// Approve only where a yes aloud would count too, Review where the card
// has to be read first, as a mail's does. iOS draws none of them, and a
// tap opens the card there.
const BUTTONS = { approve: "Approve", deny: "Deny", review: "Review" };

// Shown the moment a button is pressed, so it isn't pressed twice.
const ANSWERING = "Answering…";
const COULD_NOT = "Couldn't answer. Open Niles to answer it.";

// Browser notification reads and writes are asynchronous. Keep the
// fallback's check and replacement together so a final push cannot be
// overwritten using a snapshot of the earlier "Answering…" notification.
let displaying = Promise.resolve();
function displayNext(operation) {
  const next = displaying.then(operation);
  displaying = next.catch(() => {});
  return next;
}

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
  event.waitUntil(displayNext(() => show(message)));
});

function show(message) {
  const tag = typeof message.tag === "string" && message.tag ? message.tag : undefined;
  // A quiet one replaces a notification already there with what became
  // of it, and asks nothing.
  const quiet = message.quiet === true;
  const asking = asks(message.approval);
  const body = [typeof message.body === "string" ? message.body : "", asking?.line]
    .filter(Boolean)
    .join("\n");
  return self.registration.showNotification(message.title || "Niles", {
    body,
    icon: ICON,
    badge: ICON,
    tag,
    // A ready after a failure under the same tag replaces it, and
    // should still be heard rather than swapped in silently.
    renotify: Boolean(tag) && !quiet,
    silent: quiet,
    actions: asking ? asking.actions : [],
    // `approval` only while it can still be answered: what the buttons
    // answer, and what the open app looks for to take it off itself.
    data: { url: message.url || "/", approval: asking?.actions.length ? asking.id : undefined },
  });
}

/** What a push about something waiting for a yes asks: its buttons, and
    a line saying until when, in this device's own time. One arriving
    after that asks nothing. */
function asks(approval) {
  if (!approval || typeof approval.id !== "string" || !approval.id) return null;
  const until = typeof approval.until === "number" ? new Date(approval.until) : null;
  if (until && until.getTime() <= Date.now()) {
    return { id: approval.id, actions: [], line: "No longer waiting." };
  }
  const actions = (Array.isArray(approval.actions) ? approval.actions : [])
    .filter((action) => typeof action === "string" && Object.hasOwn(BUTTONS, action))
    .map((action) => ({ action, title: BUTTONS[action] }));
  const line = until
    ? `Waiting until ${until.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" })}`
    : undefined;
  return { id: approval.id, actions, line };
}

// A tap, or Review: Niles open somewhere is brought forward and sent to
// the notification's address — the page follows it (src/lib/pwa.ts),
// which a hash change does without reloading. With none open, one opens
// there.
self.addEventListener("notificationclick", (event) => {
  const { notification } = event;
  const data = notification.data || {};
  const url = new URL(data.url || "/", self.location.origin).href;
  const decision = event.action === "approve" ? "once" : event.action === "deny" ? "deny" : null;
  if (decision && typeof data.approval === "string") {
    event.waitUntil(answer(notification, data.approval, decision, url));
    return;
  }
  notification.close();
  event.waitUntil(open(url));
});

async function open(url) {
  const windows = await self.clients.matchAll({ type: "window", includeUncontrolled: true });
  const found = windows.find((client) => new URL(client.url).origin === self.location.origin);
  if (found) {
    await found.focus();
    found.postMessage({ type: "open", url });
    return;
  }
  await self.clients.openWindow(url);
}

/**
 * Approve or Deny, pressed on the notification. Sent the way the card's
 * own button sends it, as whoever is signed in on this device, so only
 * the approval's own person can answer it. The notification then says
 * what became of it, and if the answer didn't arrive, says so and
 * links to the card.
 */
async function answer(notification, approval, decision, url) {
  const tag = notification.tag || undefined;
  const replace = (body, more) =>
    self.registration.showNotification(notification.title, {
      body,
      icon: ICON,
      badge: ICON,
      tag,
      renotify: false,
      silent: true,
      data: { url, ...more },
    });
  await displayNext(() => replace(ANSWERING, { answering: true }));
  let said = COULD_NOT;
  try {
    const response = await fetch(`/me/approvals/${encodeURIComponent(approval)}`, {
      method: "POST",
      credentials: "same-origin",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ decision }),
    });
    if (response.ok) said = decision === "deny" ? "Denied. Nothing was done." : "Allowed.";
    else if (response.status === 404 || response.status === 409)
      said = "No longer waiting. It was answered, or it lapsed.";
  } catch {
    // Offline, or Niles out of reach. Said so, never silently.
  }
  // Niles's own push saying what became of it can land while the
  // answer is on its way. It knows more than the answer does.
  await displayNext(async () => {
    if (tag) {
      const shown = await self.registration.getNotifications({ tag });
      if (shown.some((n) => !n.data?.answering)) return;
    }
    await replace(said, {});
  });
}
