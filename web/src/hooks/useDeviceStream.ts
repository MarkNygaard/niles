import { useEffect } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { mergeReported } from "@/lib/rooms";
import type { Device, DeviceState } from "@/lib/api";

/**
 * Keep the device list in step with what the house is actually doing.
 *
 * Lights here are not only changed from here — someone says "turn the
 * kitchen off", the curve moves them at sunset, somebody presses a wall
 * switch. Polling would show all of that a few seconds late, which is
 * long enough for the page to be describing a room you are standing in
 * and can see is wrong. Niles already pushes every state change over
 * `/events/stream`, so take it from there and patch the cache in place.
 */
export function useDeviceStream() {
  const queryClient = useQueryClient();

  useEffect(() => {
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
  }, [queryClient]);
}

type WireEvent =
  | { type: "device_state_changed"; id: string; state: Partial<DeviceState> }
  | { type: "device_added"; id: string }
  | { type: "device_removed"; id: string }
  | { type: "ping" | "close" };

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
