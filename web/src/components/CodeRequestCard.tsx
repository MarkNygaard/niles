import { useId, useState } from "react";
import { ShieldCheck } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import type { CodeRequest } from "@/lib/api";

/**
 * A turn's ask for the code a site sent while Niles signed the person
 * in. The code goes from this field to Niles's browser, never into the
 * chat.
 */
export function CodeRequestCard({
  request,
  onEnter,
  entering,
}: {
  request: CodeRequest;
  /** Called with the typed code; it reaches the page and nothing else. */
  onEnter?: (code: string) => void;
  /** The conversation is carrying on after an enter. */
  entering?: boolean;
}) {
  const id = useId();
  const [code, setCode] = useState("");
  const waiting = request.status === "waiting";
  return (
    <section
      aria-label={`${request.label} sent you a code`}
      data-code-request={request.id}
      className="bg-muted/60 flex w-full max-w-xl flex-col gap-3 self-start rounded-2xl p-4 ring-1 ring-foreground/10"
    >
      <div className="flex items-start gap-3">
        <ShieldCheck aria-hidden className="mt-0.5 size-5 shrink-0" />
        <div className="min-w-0">
          <h3 className="text-base leading-snug font-medium">{request.label} sent you a code</h3>
          <p className="text-muted-foreground font-mono text-sm break-all">{request.site}</p>
        </div>
      </div>
      {waiting ? (
        <form
          className="flex flex-col gap-3"
          onSubmit={(event) => {
            event.preventDefault();
            const typed = code.trim();
            if (!typed) return;
            setCode("");
            onEnter?.(typed);
          }}
        >
          <div className="flex flex-col gap-1.5">
            <Label htmlFor={id}>Code</Label>
            <Input
              id={id}
              value={code}
              onChange={(event) => setCode(event.target.value)}
              autoComplete="one-time-code"
              autoCapitalize="off"
              autoCorrect="off"
              spellCheck={false}
              maxLength={40}
              disabled={entering}
            />
          </div>
          <Button
            type="submit"
            className="rounded-full px-5 self-start"
            disabled={entering || code.trim() === ""}
          >
            Enter code
          </Button>
          <p className="text-muted-foreground text-sm">
            It goes straight to {request.site}, never into the chat.
          </p>
        </form>
      ) : (
        <p role="status" className="text-muted-foreground text-sm">
          {request.status === "entered" ? "Code entered" : "Lapsed. Nothing was typed."}
        </p>
      )}
    </section>
  );
}
