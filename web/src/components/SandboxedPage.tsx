import { useEffect, useRef, useState } from "react";
import { ExternalLink } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogBody,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  NO_FEATURES,
  PageSaves,
  SANDBOX,
  buildSrcdoc,
  openableUrl,
  readMessage,
  readTheme,
} from "@/lib/sandboxedPage";
import { useDarkMode } from "@/lib/theme";

/** How long a page has to say it is ready before the built-in view is
    shown instead. Everything in it is inline, so a page that works is
    ready in a blink. */
export const READY_WITHIN_MS = 4000;

/** The height the frame has before the page says its own. */
const FIRST_HEIGHT = 240;

export interface SandboxedPageProps {
  /** The page. A new one is a new frame: give each its own `key`. */
  html: string;
  /** The data it draws, handed again whenever it changes. */
  content: unknown;
  /** The house's today, YYYY-MM-DD. */
  today: string;
  /** What the frame is, for a screen reader. */
  title: string;
  /** Content the page saved, batched; without it the page is read-only
      (an older version open), and its saves go nowhere. */
  onSave?: (content: unknown, reason: string) => void;
  /** The page failed, or never started: show the built-in view. */
  onFallback: (why: string) => void;
}

/**
 * An artifact's page in its sealed frame. See `lib/sandboxedPage.ts`
 * for what the frame is given and may say; this is the frame's life:
 * the data and theme sent in as they change, its height taken from it,
 * a link it asks for opened only when the person agrees, its saves
 * batched, and the built-in view called for when it breaks.
 */
export function SandboxedPage({
  html,
  content,
  today,
  title,
  onSave,
  onFallback,
}: SandboxedPageProps) {
  const frame = useRef<HTMLIFrameElement>(null);
  // The document is built once, with the data as it is when the page
  // opens; later data goes in by message, so the frame is not reloaded
  // (and the person's place in it lost) each time a run is marked.
  const [srcdoc] = useState(() =>
    buildSrcdoc({ html, data: content, theme: readTheme(), today }),
  );
  const [height, setHeight] = useState(FIRST_HEIGHT);
  const [ready, setReady] = useState(false);
  const [asking, setAsking] = useState<URL>();
  const dark = useDarkMode();

  const live = useRef({ onSave, onFallback, asking });
  useEffect(() => {
    live.current = { onSave, onFallback, asking };
  });

  const [saves] = useState(
    () => new PageSaves((c, reason) => live.current.onSave?.(c, reason)),
  );

  const post = (message: Record<string, unknown>) =>
    frame.current?.contentWindow?.postMessage({ niles: 1, ...message }, "*");

  useEffect(() => {
    const listen = (event: MessageEvent) => {
      const m = readMessage(event, frame.current?.contentWindow);
      if (!m) return;
      switch (m.type) {
        case "ready":
          setReady(true);
          break;
        case "height":
          setHeight(Math.max(m.height, 24));
          break;
        case "error":
          live.current.onFallback(`it stopped with an error (${m.message})`);
          break;
        case "open": {
          const url = openableUrl(m.url, window.location.origin);
          if (url && !live.current.asking) setAsking(url);
          break;
        }
        case "save":
          if (live.current.onSave) saves.push(m.content, m.reason);
          break;
      }
    };
    window.addEventListener("message", listen);
    return () => window.removeEventListener("message", listen);
  }, [saves]);

  useEffect(() => {
    if (ready) return;
    const timer = setTimeout(
      () => live.current.onFallback("it did not start"),
      READY_WITHIN_MS,
    );
    return () => clearTimeout(timer);
  }, [ready]);

  // The first data is in the document; what comes after is sent.
  const sent = useRef(content);
  useEffect(() => {
    if (!ready || sent.current === content) return;
    sent.current = content;
    post({ type: "data", data: content, today });
  }, [ready, content, today]);

  useEffect(() => {
    if (ready) post({ type: "theme", theme: readTheme() });
  }, [ready, dark]);

  useEffect(() => {
    const hidden = () => {
      if (document.visibilityState === "hidden") saves.flush();
    };
    document.addEventListener("visibilitychange", hidden);
    return () => {
      document.removeEventListener("visibilitychange", hidden);
      saves.flush();
    };
  }, [saves]);

  return (
    <>
      <iframe
        ref={frame}
        title={title}
        sandbox={SANDBOX}
        srcDoc={srcdoc}
        referrerPolicy="no-referrer"
        allow={NO_FEATURES}
        // Chrome's: the frame gets fresh, empty storage of its own, on
        // top of the opaque origin the sandbox already gives it.
        {...{ credentialless: "" }}
        className="block w-full rounded-lg border-0"
        style={{ height }}
      />
      <Dialog open={asking !== undefined} onOpenChange={(open) => !open && setAsking(undefined)}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Open a link from this page?</DialogTitle>
          </DialogHeader>
          <DialogBody className="flex flex-col gap-3">
            <DialogDescription>
              It goes to <strong className="text-foreground">{asking?.hostname}</strong>. The page
              was written by an assistant, so open it only if that is where you expect it to go.
            </DialogDescription>
            <p className="text-muted-foreground text-xs break-all">{asking?.href}</p>
            <div className="flex justify-end gap-2">
              <Button variant="outline" onClick={() => setAsking(undefined)}>
                Cancel
              </Button>
              <Button
                onClick={() => {
                  if (asking) window.open(asking.href, "_blank", "noopener,noreferrer");
                  setAsking(undefined);
                }}
              >
                <ExternalLink />
                Open
              </Button>
            </div>
          </DialogBody>
        </DialogContent>
      </Dialog>
    </>
  );
}
