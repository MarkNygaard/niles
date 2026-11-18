import { useCallback, useEffect, useRef, useState } from "react";
import { liveUrl } from "@/lib/browser";
import type { LiveInput } from "@/lib/browser";

/**
 * The newest picture of the person's browser over the live socket, and
 * a way to send it input. Reconnects while `enabled`, so a session that
 * ends and a new one that opens are both followed. Every view of it
 * shares one screencast on the server; each has its own socket.
 */
export function useLivePicture(enabled: boolean) {
  const [picture, setPicture] = useState<string | null>(null);
  const [refused, setRefused] = useState<string | null>(null);
  const socket = useRef<WebSocket | null>(null);

  useEffect(() => {
    if (!enabled) return;
    let closed = false;
    let retry: ReturnType<typeof setTimeout> | undefined;
    const connect = () => {
      const ws = new WebSocket(liveUrl(window.location));
      ws.binaryType = "blob";
      ws.onmessage = (message) => {
        if (message.data instanceof Blob) {
          const next = URL.createObjectURL(message.data);
          setPicture((shown) => {
            if (shown) URL.revokeObjectURL(shown);
            return next;
          });
          return;
        }
        try {
          const said = JSON.parse(String(message.data)) as { refused?: string };
          if (said.refused) setRefused(said.refused);
        } catch {
          // Not anything the view says.
        }
      };
      ws.onclose = () => {
        if (!closed) retry = setTimeout(connect, 2_000);
      };
      socket.current = ws;
    };
    connect();
    return () => {
      closed = true;
      clearTimeout(retry);
      socket.current?.close();
      socket.current = null;
      setPicture((shown) => {
        if (shown) URL.revokeObjectURL(shown);
        return null;
      });
    };
  }, [enabled]);

  const send = useCallback((inputs: LiveInput[]) => {
    const ws = socket.current;
    if (ws?.readyState !== WebSocket.OPEN) return;
    for (const input of inputs) ws.send(JSON.stringify(input));
  }, []);

  return { picture, refused, clearRefused: () => setRefused(null), send };
}
