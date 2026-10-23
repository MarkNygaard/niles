import { useEffect } from "react";
import { useQueryClient } from "@tanstack/react-query";
import type { QueryClient } from "@tanstack/react-query";
import { mergeReported } from "@/lib/rooms";
import { APPROVALS, approvalTag } from "@/lib/approvals";
import { closeWaitingLater } from "@/lib/pwa";
import { ARTIFACTS, ARTIFACT_LIST } from "@/lib/artifacts";
import { NOTIFICATIONS } from "@/lib/notifications";
import type { Artifact, ArtifactList, ArtifactStatus, Device, DeviceState } from "@/lib/api";

/**
 * Keep the device list in step with what the house is actually doing.
 *
 * Lights here are not only changed from here — someone says "turn the
 * kitchen off", the curve moves them at sunset, somebody presses a wall
 * switch. Polling would show all of that a few seconds late, which is
 * long enough for the page to be describing a room you are standing in
 * and can see is wrong. Niles already pushes every state change over
 * `/events/stream`, so take it from there and patch the cache in place.
 *
 * The same stream says when a plugin's page or widget has changed,
 * which refetches it if it is on screen, and when one of the signed-in
 * person's artifacts moves on, which Saved and the status line follow,
 * when something of theirs waiting for a yes is asked or answered —
 * and once it is answered or lapses, its notification is taken off this
 * device after Niles's own push had its chance to replace it — and
 * when a notification of theirs is kept or read, which the bell follows.
 *
 * One socket however many components ask: the bell in the header asks
 * on every page, and so does the page under it.
 */
export function useDeviceStream() {
  const queryClient = useQueryClient();

  useEffect(() => {
    const shared = streams.get(queryClient) ?? { users: 0, stop: () => {} };
    if (shared.users === 0) shared.stop = stream(queryClient);
    shared.users += 1;
    streams.set(queryClient, shared);
    return () => {
      shared.users -= 1;
      if (shared.users === 0) {
        shared.stop();
        streams.delete(queryClient);
      }
    };
  }, [queryClient]);
}

const streams = new Map<QueryClient, { users: number; stop: () => void }>();

/** The socket, opened again whenever it drops, until the answer is called. */
function stream(queryClient: QueryClient): () => void {
  let socket: WebSocket | null = null;
  let retry: ReturnType<typeof setTimeout> | undefined;
  let attempt = 0;
  let closed = false;

  function connect() {
    if (closed) return;
    const protocol = window.location.protocol === "https:" ? "wss:" : "ws:";
    socket = new WebSocket(`${protocol}//${window.location.host}/events/stream`);

    socket.onopen = () => {
      attempt = 0;
    };

    socket.onmessage = (message) => {
      const event = parse(message.data);
      if (!event) return;
      switch (event.type) {
        case "device_state_changed":
          patchState(queryClient, event.id, event.state);
          break;
        // A device appearing or going away changes the shape of the
        // list rather than one row of it, and the frame carries only
        // the id, so ask for the list again.
        case "device_added":
        case "device_removed":
          queryClient.invalidateQueries({ queryKey: ["devices"] });
          break;
        // A plugin has something new for a page or widget. Every
        // room's copy is marked stale, and only those on screen are
        // asked for again: nothing polls for this.
        case "ui_invalidated":
          queryClient.invalidateQueries({
            queryKey: ["surface", event.plugin, event.surface],
          });
          break;
        case "artifact_changed":
          patchArtifact(queryClient, event);
          void queryClient.invalidateQueries({ queryKey: ARTIFACTS });
          // A finished job leaves a card in the chat it came from.
          if (event.status === "ready" || event.status === "failed") {
            void queryClient.invalidateQueries({ queryKey: ["chat"] });
          }
          break;
        // Asked, answered or lapsed: the panel's lists and count, and
        // the card in the conversation it was asked in.
        case "approval_changed":
          void queryClient.invalidateQueries({ queryKey: APPROVALS });
          void queryClient.invalidateQueries({ queryKey: ["chat"] });
          // Answered elsewhere or lapsed: Niles's push replaces its
          // notification here with what became of it, and should that
          // push not come, the open app takes the waiting one off itself.
          if (event.status !== "pending") closeWaitingLater(approvalTag(event.id));
          break;
        // Kept, or read on another device: the bell and the page.
        case "notification_added":
        case "notifications_read":
          void queryClient.invalidateQueries({ queryKey: NOTIFICATIONS });
          break;
      }
    };

    socket.onclose = () => {
      if (closed) return;
      // A pod restart drops every socket at once. Backing off keeps a
      // browser left open overnight from hammering a Niles that is
      // still coming up.
      const delay = Math.min(1_000 * 2 ** attempt, 30_000);
      attempt += 1;
      retry = setTimeout(connect, delay);
    };
  }

  connect();
  return () => {
    closed = true;
    clearTimeout(retry);
    socket?.close();
  };
}

type WireEvent =
  | { type: "device_state_changed"; id: string; state: Partial<DeviceState> }
  | { type: "device_added"; id: string }
  | { type: "device_removed"; id: string }
  | { type: "ui_invalidated"; plugin: string; surface: string }
  | ArtifactChanged
  | { type: "approval_changed"; id: string; status: string }
  | { type: "notification_added"; id: number }
  | { type: "notifications_read" }
  | { type: "ping" | "close" };

interface ArtifactChanged {
  type: "artifact_changed";
  id: number;
  kind: string;
  title: string;
  status: ArtifactStatus | "deleted";
  status_text: string | null;
}

/**
 * Saved as the frame says it is now, before the refetch lands: the
 * status line says "Writing…" the moment a job starts, not a round trip
 * later. One the list has not heard of yet joins it from the frame.
 */
function patchArtifact(queryClient: QueryClient, event: ArtifactChanged) {
  queryClient.setQueryData<ArtifactList>(ARTIFACT_LIST, (list) => {
    if (!list) return list;
    if (event.status === "deleted") {
      return { ...list, artifacts: list.artifacts.filter((a) => a.id !== event.id) };
    }
    const now = new Date().toISOString();
    const status = event.status;
    const known = list.artifacts.some((a) => a.id === event.id);
    const artifacts: Artifact[] = known
      ? list.artifacts.map((a) =>
          a.id === event.id
            ? { ...a, title: event.title, status, status_text: event.status_text, updated_at: now }
            : a,
        )
      : [
          {
            id: event.id,
            kind: event.kind,
            title: event.title,
            summary: "",
            pinned: false,
            archived: false,
            notify: true,
            status,
            status_text: event.status_text,
            error: null,
            created_at: now,
            updated_at: now,
            current_revision: 0,
            preview: { lines: [] },
          },
          ...list.artifacts,
        ];
    return { ...list, artifacts };
  });
}

function parse(raw: unknown): WireEvent | null {
  if (typeof raw !== "string") return null;
  try {
    return JSON.parse(raw) as WireEvent;
  } catch {
    // A frame we can't read is not worth tearing the stream down for.
    return null;
  }
}

function patchState(
  queryClient: ReturnType<typeof useQueryClient>,
  id: string,
  state: Partial<DeviceState>,
) {
  queryClient.setQueryData<Device[]>(["devices"], (devices) =>
    devices?.map((device) =>
      device.id === id
        ? { ...device, state: mergeReported(device.state, state) }
        : device,
    ),
  );
}
