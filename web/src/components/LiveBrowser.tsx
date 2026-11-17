import { useCallback, useEffect, useRef, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ArrowLeft, CornerDownLeft, Delete, Hand, RotateCw } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Skeleton } from "@/components/ui/skeleton";
import { api } from "@/lib/api";
import type { GiveBackThen, LiveView } from "@/lib/api";
import {
  BROWSER,
  DRAG,
  dragWheel,
  keyOf,
  liveUrl,
  modifiersOf,
  pageAt,
  tap,
} from "@/lib/browser";
import type { LiveInput } from "@/lib/browser";

/** The page's pictures over the live socket, and a way to send it input.
    Reconnects while `enabled`, so a session that ends and a new one that
    opens are both followed. */
function useLiveSocket(enabled: boolean) {
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

const BUTTONS = ["left", "middle", "right"] as const;

/** A finger on the page: a tap is a click where it went down, a drag
    scrolls. */
interface Touch {
  id: number;
  start: { x: number; y: number };
  last: { x: number; y: number };
  dragging: boolean;
}

/**
 * Niles's browser, live: what it shows as it works, and, once taken
 * over, the person's own mouse, keys and touch on it. Given back, Niles
 * carries on in the conversation open beside it.
 */
export function LiveBrowser({
  onGiveBack,
  givingBack,
}: {
  onGiveBack: (then: GiveBackThen) => void;
  givingBack: boolean;
}) {
  const queryClient = useQueryClient();
  const view = useQuery({ queryKey: BROWSER, queryFn: api.browser, retry: false });
  const settle = () => queryClient.invalidateQueries({ queryKey: BROWSER });
  const take = useMutation({ mutationFn: api.takeBrowser, onSettled: settle });
  const back = useMutation({ mutationFn: api.browserBack });
  const reload = useMutation({ mutationFn: api.browserReload });
  const live = view.data ?? null;
  const { picture, refused, clearRefused, send } = useLiveSocket(live !== null);

  if (view.isPending) return <Skeleton className="aspect-[1280/900] w-full" />;
  if (view.isError) return <p className="text-destructive text-sm">{view.error.message}</p>;
  if (live === null) {
    return (
      <p className="text-muted-foreground text-sm">
        Nothing is open in Niles's browser. When Niles browses for you, you can watch it here
        and take it over.
      </p>
    );
  }

  const working = live.control === "person";
  const error = take.error ?? back.error ?? reload.error;
  return (
    <div className="flex flex-col gap-3">
      <div className="flex flex-col gap-1">
        <p className="text-muted-foreground truncate text-xs" title={live.address}>
          {live.address}
        </p>
        <p className="text-sm">
          {working ? "You have the browser. Niles waits until you give it back." : "Niles is using the browser."}
        </p>
        {live.handoff && working && <p className="text-sm font-medium">{live.handoff.why}</p>}
      </div>
      <Page live={live} picture={picture} working={working} send={send} />
      {refused === "not_in_control" && (
        <p className="text-muted-foreground text-xs">
          Niles has the browser again.{" "}
          <button type="button" className="underline" onClick={clearRefused}>
            OK
          </button>
        </p>
      )}
      {working && <Typing send={send} />}
      {error && <p className="text-destructive text-sm">{error.message}</p>}
      <div className="flex flex-wrap gap-2">
        {working ? (
          <>
            <Button
              variant="outline"
              size="sm"
              aria-label="Back"
              disabled={back.isPending}
              onClick={() => back.mutate()}
            >
              <ArrowLeft aria-hidden />
            </Button>
            <Button
              variant="outline"
              size="sm"
              aria-label="Reload"
              disabled={reload.isPending}
              onClick={() => reload.mutate()}
            >
              <RotateCw aria-hidden />
            </Button>
            <Button size="sm" disabled={givingBack} onClick={() => onGiveBack("carry_on")}>
              Give back to Niles
            </Button>
            <Button
              variant="ghost"
              size="sm"
              disabled={givingBack}
              onClick={() => onGiveBack("stop")}
            >
              Stop
            </Button>
          </>
        ) : (
          <Button size="sm" disabled={take.isPending} onClick={() => take.mutate()}>
            <Hand aria-hidden />
            Take over
          </Button>
        )}
      </div>
    </div>
  );
}

/** The picture of the page, and while the person has the browser, the
    input on it. */
function Page({
  live,
  picture,
  working,
  send,
}: {
  live: LiveView;
  picture: string | null;
  working: boolean;
  send: (inputs: LiveInput[]) => void;
}) {
  const shown = useRef<HTMLDivElement>(null);
  const touch = useRef<Touch | null>(null);
  const moved = useRef(0);

  const at = (e: { clientX: number; clientY: number }) => {
    const box = shown.current?.getBoundingClientRect();
    return box ? pageAt(e.clientX, e.clientY, box, live) : null;
  };

  return (
    <div
      ref={shown}
      tabIndex={working ? 0 : -1}
      aria-label={working ? "The page, yours to use" : "The page Niles is using"}
      className="border-border relative aspect-[1280/900] w-full overflow-hidden rounded-md border bg-white outline-none focus-visible:ring-3 focus-visible:ring-ring/50"
      style={{ touchAction: working ? "none" : undefined }}
      // The panel's sheet on a phone swipes away on a drag. While the
      // page is the person's, a drag on it is theirs.
      data-base-ui-swipe-ignore={working ? "" : undefined}
      onPointerDown={(e) => {
        if (!working) return;
        const point = at(e);
        if (!point) return;
        e.currentTarget.focus();
        e.currentTarget.setPointerCapture(e.pointerId);
        if (e.pointerType === "mouse") {
          send([
            {
              type: "press",
              ...point,
              press: "down",
              button: BUTTONS[e.button] ?? "left",
              clicks: Math.min(Math.max(e.detail, 1), 3),
            },
          ]);
        } else {
          touch.current = { id: e.pointerId, start: point, last: point, dragging: false };
        }
      }}
      onPointerMove={(e) => {
        if (!working) return;
        const point = at(e);
        if (!point) return;
        if (e.pointerType === "mouse") {
          // Enough for hover menus without a message per pixel.
          const now = performance.now();
          if (now - moved.current < 50) return;
          moved.current = now;
          send([{ type: "move", ...point }]);
          return;
        }
        const finger = touch.current;
        if (!finger || finger.id !== e.pointerId) return;
        const far = Math.hypot(point.x - finger.start.x, point.y - finger.start.y);
        if (!finger.dragging && far < DRAG) return;
        finger.dragging = true;
        send([{ type: "wheel", ...finger.start, ...dragWheel(finger.last, point) }]);
        finger.last = point;
      }}
      onPointerUp={(e) => {
        if (!working) return;
        if (e.pointerType === "mouse") {
          const point = at(e);
          if (point) {
            send([
              {
                type: "press",
                ...point,
                press: "up",
                button: BUTTONS[e.button] ?? "left",
                clicks: Math.min(Math.max(e.detail, 1), 3),
              },
            ]);
          }
          return;
        }
        const finger = touch.current;
        touch.current = null;
        if (!finger || finger.dragging) return;
        const { x, y } = finger.start;
        send([
          { type: "move", x, y },
          { type: "press", x, y, press: "down", button: "left", clicks: 1 },
          { type: "press", x, y, press: "up", button: "left", clicks: 1 },
        ]);
      }}
      onPointerCancel={() => {
        touch.current = null;
      }}
      onWheel={(e) => {
        if (!working) return;
        const point = at(e);
        if (point) send([{ type: "wheel", ...point, dx: e.deltaX, dy: e.deltaY }]);
      }}
      onKeyDown={(e) => {
        if (!working) return;
        const key = keyOf(e.key);
        if (!key) return;
        e.preventDefault();
        send([{ type: "key", press: "down", key, modifiers: modifiersOf(e) }]);
      }}
      onKeyUp={(e) => {
        if (!working) return;
        const key = keyOf(e.key);
        if (!key) return;
        e.preventDefault();
        send([{ type: "key", press: "up", key, modifiers: modifiersOf(e) }]);
      }}
      onContextMenu={(e) => working && e.preventDefault()}
    >
      {picture ? (
        <img
          src={picture}
          alt={`What ${live.address} shows`}
          draggable={false}
          className="pointer-events-none h-full w-full select-none object-contain"
        />
      ) : (
        <Skeleton className="h-full w-full" />
      )}
    </div>
  );
}

/** Typing for a phone, whose own keyboard only a text field brings up. */
function Typing({ send }: { send: (inputs: LiveInput[]) => void }) {
  const [text, setText] = useState("");
  return (
    <form
      className="flex flex-wrap items-center gap-2"
      onSubmit={(e) => {
        e.preventDefault();
        if (!text) return;
        send([{ type: "text", text }]);
        setText("");
      }}
    >
      <Input
        aria-label="Type into the page"
        placeholder="Type into the page"
        autoComplete="off"
        autoCapitalize="off"
        value={text}
        maxLength={1000}
        className="min-w-0 flex-1"
        onChange={(e) => setText(e.target.value)}
      />
      <Button type="submit" size="sm" variant="outline" disabled={!text}>
        Send
      </Button>
      <Button
        type="button"
        size="sm"
        variant="outline"
        aria-label="Enter"
        onClick={() => send(tap("Enter"))}
      >
        <CornerDownLeft aria-hidden />
      </Button>
      <Button
        type="button"
        size="sm"
        variant="outline"
        aria-label="Backspace"
        onClick={() => send(tap("Backspace"))}
      >
        <Delete aria-hidden />
      </Button>
      <Button type="button" size="sm" variant="outline" onClick={() => send(tap("Tab"))}>
        Tab
      </Button>
    </form>
  );
}
