import { useEffect, useRef, useState } from "react";
import { Camera, ImageUp } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogBody,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from "@/components/ui/dialog";
import { authenticatorFrom, decodeImage, decodePixels } from "@/lib/qr";

/**
 * An authenticator key from the site's setup QR code: scanned with the
 * camera, or read from a picked image or a pasted screenshot. Decoded
 * on this device; only the link inside goes to the form, and the
 * picture and the camera feed go nowhere.
 */
export function QrKey({ onKey }: { onKey: (link: string) => void }) {
  const picker = useRef<HTMLInputElement>(null);
  const root = useRef<HTMLDivElement>(null);
  const [scanning, setScanning] = useState(false);
  const [said, setSaid] = useState<{ ok: boolean; text: string } | null>(null);

  const read = (text: string | null) => {
    if (text === null) {
      setSaid({ ok: false, text: "No QR code found in that picture." });
      return;
    }
    const got = authenticatorFrom(text);
    if ("refused" in got) {
      setSaid({ ok: false, text: got.refused });
      return;
    }
    onKey(got.link);
    setSaid({ ok: true, text: "Read the key from the QR code." });
  };

  const fromImage = async (file: Blob | undefined) => {
    if (!file) return;
    try {
      read(await decodeImage(file));
    } catch {
      setSaid({ ok: false, text: "That picture couldn't be read." });
    }
  };

  // A screenshot pasted into this form, as into its key field. Only this
  // form's: the chat's own field takes pictures too.
  useEffect(() => {
    const paste = (e: ClipboardEvent) => {
      const form = root.current?.closest("form");
      if (!form || !(e.target instanceof Node) || !form.contains(e.target)) return;
      const image = [...(e.clipboardData?.items ?? [])].find((i) => i.type.startsWith("image/"));
      if (!image) return;
      e.preventDefault();
      void fromImage(image.getAsFile() ?? undefined);
    };
    window.addEventListener("paste", paste);
    return () => window.removeEventListener("paste", paste);
  });

  return (
    <div ref={root} className="flex flex-col gap-1.5">
      <div className="flex flex-wrap gap-2">
        <Button type="button" variant="outline" size="sm" onClick={() => setScanning(true)}>
          <Camera aria-hidden />
          Scan QR code
        </Button>
        <Button type="button" variant="outline" size="sm" onClick={() => picker.current?.click()}>
          <ImageUp aria-hidden />
          From an image
        </Button>
        <input
          ref={picker}
          type="file"
          accept="image/*"
          aria-label="QR code image"
          className="hidden"
          onChange={(e) => {
            void fromImage(e.target.files?.[0]);
            e.target.value = "";
          }}
        />
      </div>
      {said && (
        <p role="status" className={said.ok ? "text-sm" : "text-destructive text-sm"}>
          {said.text}
        </p>
      )}
      <Dialog open={scanning} onOpenChange={(open: boolean) => setScanning(open)}>
        <DialogContent className="gap-0 p-0">
          <div className="border-border border-b px-4 py-3">
            <DialogTitle>Scan the QR code</DialogTitle>
            <DialogDescription>
              Point the camera at the code the site shows. It's read on this device.
            </DialogDescription>
          </div>
          <DialogBody>
            {scanning && (
              <Scanner
                onText={(text) => {
                  setScanning(false);
                  read(text);
                }}
                onFail={(why) => {
                  setScanning(false);
                  setSaid({ ok: false, text: why });
                }}
              />
            )}
          </DialogBody>
        </DialogContent>
      </Dialog>
    </div>
  );
}

/** The camera, read a few times a second until a QR code shows. */
function Scanner({
  onText,
  onFail,
}: {
  onText: (text: string) => void;
  onFail: (why: string) => void;
}) {
  const video = useRef<HTMLVideoElement>(null);
  // The latest callbacks, so the camera starts once and isn't restarted
  // by a render.
  const said = useRef({ onText, onFail });
  said.current = { onText, onFail };

  useEffect(() => {
    let stream: MediaStream | null = null;
    let timer: ReturnType<typeof setInterval> | undefined;
    let done = false;
    const canvas = document.createElement("canvas");
    const stop = () => {
      done = true;
      clearInterval(timer);
      stream?.getTracks().forEach((t) => t.stop());
    };
    if (!navigator.mediaDevices?.getUserMedia) {
      said.current.onFail("This browser can't use the camera here. Try From an image.");
      return stop;
    }
    navigator.mediaDevices
      .getUserMedia({ video: { facingMode: "environment" }, audio: false })
      .then((got) => {
        if (done) {
          got.getTracks().forEach((t) => t.stop());
          return;
        }
        stream = got;
        const el = video.current;
        if (!el) return;
        el.srcObject = got;
        void el.play();
        timer = setInterval(() => {
          if (!el.videoWidth) return;
          canvas.width = el.videoWidth;
          canvas.height = el.videoHeight;
          const context = canvas.getContext("2d", { willReadFrequently: true });
          if (!context) return;
          context.drawImage(el, 0, 0);
          const pixels = context.getImageData(0, 0, canvas.width, canvas.height);
          const text = decodePixels(pixels.data, canvas.width, canvas.height);
          if (text !== null) {
            stop();
            said.current.onText(text);
          }
        }, 250);
      })
      .catch(() =>
        said.current.onFail("Niles couldn't use the camera. Allow it, or try From an image."),
      );
    return stop;
  }, []);

  return (
    <video
      ref={video}
      muted
      playsInline
      aria-label="The camera"
      className="aspect-square w-full rounded-md bg-black object-cover"
    />
  );
}
