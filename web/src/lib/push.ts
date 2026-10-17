/**
 * Notifications on this device, by the browser's own Web Push.
 *
 * The server keeps who gets what (crates/niles-webpush); this is the
 * device's half: whether it can be notified at all, asking the person,
 * and handing the subscription over.
 */
import { api } from "@/lib/api";

/**
 * What this device can do about notifications.
 *
 * - `needs-home-screen`: an iPhone or iPad in Safari. iOS notifies only
 *   the app added to the Home Screen (16.4 and later), so a switch here
 *   could never work.
 * - `unsupported`: no Web Push in this browser at all.
 * - `denied`: the person said no, and only the browser's settings can
 *   undo that.
 * - `available`: can be asked, or already said yes.
 */
export type PushSupport = "available" | "denied" | "needs-home-screen" | "unsupported";

/** What the answer depends on, read off the browser by `environment`. */
export interface PushEnvironment {
  userAgent: string;
  /** An iPad says it is a Mac; only its touch screen gives it away. */
  maxTouchPoints: number;
  /** Opened from the Home Screen rather than in a browser tab. */
  standalone: boolean;
  /** A service worker, a push manager and notifications, all three. */
  hasPush: boolean;
  permission: NotificationPermission | undefined;
}

export function isAppleMobile(userAgent: string, maxTouchPoints: number): boolean {
  if (/iPhone|iPad|iPod/.test(userAgent)) return true;
  return /Macintosh/.test(userAgent) && maxTouchPoints > 1;
}

export function pushSupport(env: PushEnvironment): PushSupport {
  // Before `hasPush`: Safari in a tab has no PushManager, and "this
  // browser can't" would send the person looking for another browser
  // when adding the app to the Home Screen is the whole answer.
  if (isAppleMobile(env.userAgent, env.maxTouchPoints) && !env.standalone) {
    return "needs-home-screen";
  }
  if (!env.hasPush) return "unsupported";
  if (env.permission === "denied") return "denied";
  return "available";
}

/** Where to turn notifications back on after saying no. */
export function deniedHelp(env: Pick<PushEnvironment, "userAgent" | "maxTouchPoints">): string {
  if (isAppleMobile(env.userAgent, env.maxTouchPoints)) {
    return "Notifications for Niles are turned off on this device. Turn them on in Settings → Notifications → Niles, then come back here.";
  }
  if (/Android/.test(env.userAgent)) {
    return "Notifications for Niles are blocked on this device. Long-press the Niles icon, open App info → Notifications and allow them, then come back here.";
  }
  return "Notifications for Niles are blocked in this browser. Allow them in the site settings by the address bar, then come back here.";
}

export function environment(): PushEnvironment {
  const nav = navigator as Navigator & { standalone?: boolean };
  let standalone = nav.standalone === true;
  try {
    standalone ||= window.matchMedia("(display-mode: standalone)").matches;
  } catch {
    // No matchMedia: a tab, as far as anyone can tell.
  }
  const hasPush =
    "serviceWorker" in navigator && "PushManager" in window && "Notification" in window;
  return {
    userAgent: nav.userAgent,
    maxTouchPoints: nav.maxTouchPoints ?? 0,
    standalone,
    hasPush,
    permission: "Notification" in window ? Notification.permission : undefined,
  };
}

/** Base64url, as the server writes its key, to the bytes
    `applicationServerKey` takes. */
export function base64UrlToBytes(value: string): Uint8Array<ArrayBuffer> {
  const base64 = value.trim().replace(/-/g, "+").replace(/_/g, "/");
  const padded = base64 + "=".repeat((4 - (base64.length % 4)) % 4);
  const raw = atob(padded);
  const bytes = new Uint8Array(raw.length);
  for (let i = 0; i < raw.length; i++) bytes[i] = raw.charCodeAt(i);
  return bytes;
}

/** Whether a subscription was made with `key`. A subscription made with
    another key (the database was replaced) can never be pushed to. */
export function sameKey(made: ArrayBuffer | null | undefined, key: Uint8Array): boolean {
  if (!made) return false;
  const bytes = new Uint8Array(made);
  return bytes.length === key.length && bytes.every((b, i) => b === key[i]);
}

/** The person said no, or dismissed the browser's question. */
export class PushRefused extends Error {
  constructor(readonly permission: NotificationPermission) {
    super(
      permission === "denied"
        ? "Notifications are blocked for Niles on this device."
        : "Notifications weren't allowed. Try again, and choose Allow.",
    );
  }
}

/** This device's subscription, if it has one. `getRegistration` rather
    than `ready`, which waits forever on a page with no worker. */
export async function currentSubscription(): Promise<PushSubscription | null> {
  if (!("serviceWorker" in navigator)) return null;
  const registration = await navigator.serviceWorker.getRegistration();
  return (await registration?.pushManager.getSubscription()) ?? null;
}

/**
 * Ask, subscribe and tell Niles. Call it straight from a tap: Safari
 * shows its question only to a page acting on one, which is also why
 * the key is fetched beforehand and passed in rather than awaited here
 * first.
 */
export async function turnOn(key: string): Promise<PushSubscription> {
  const permission = await Notification.requestPermission();
  if (permission !== "granted") throw new PushRefused(permission);
  const registration = await navigator.serviceWorker.ready;
  const applicationServerKey = base64UrlToBytes(key);
  let subscription = await registration.pushManager.getSubscription();
  if (subscription && !sameKey(subscription.options.applicationServerKey, applicationServerKey)) {
    await subscription.unsubscribe();
    subscription = null;
  }
  subscription ??= await registration.pushManager.subscribe({
    userVisibleOnly: true,
    applicationServerKey,
  });
  await api.subscribePush(subscription.toJSON());
  return subscription;
}

/** Unsubscribed here first: if telling Niles fails, the push service
    answers its next try with "gone", and Niles forgets it then. */
export async function turnOff(): Promise<void> {
  const subscription = await currentSubscription();
  if (!subscription) return;
  const { endpoint } = subscription;
  await subscription.unsubscribe();
  await api.unsubscribePush(endpoint);
}

const NUDGE_DISMISSED = "niles-push-nudge-dismissed";

export function nudgeDismissed(): boolean {
  try {
    return localStorage.getItem(NUDGE_DISMISSED) === "1";
  } catch {
    return false;
  }
}

export function dismissNudge(): void {
  try {
    localStorage.setItem(NUDGE_DISMISSED, "1");
  } catch {
    // Private mode: offered again next time, which is the lesser harm.
  }
}

/** Whether to offer "Notify me when it's done" under a job. */
export function shouldNudge(state: {
  support: PushSupport;
  /** The server can send: it has a database and its secrets key. */
  serverReady: boolean;
  on: boolean;
  dismissed: boolean;
}): boolean {
  return state.support === "available" && state.serverReady && !state.on && !state.dismissed;
}
