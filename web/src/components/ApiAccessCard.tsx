import { useEffect, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Check, Copy, Eye, EyeOff } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import {
  Dialog,
  DialogBody,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from "@/components/ui/dialog";
import { Skeleton } from "@/components/ui/skeleton";
import { api } from "@/lib/api";
import type { ApiToken } from "@/lib/api";

export const API_TOKEN = ["api-token"];

/**
 * The key scripts and other systems use, made by Niles.
 *
 * Shown, unlike every other secret, because the point of it is to be
 * pasted somewhere — but masked until asked, since it is as good as a
 * signed-in person to whoever reads it off the screen.
 */
export function ApiAccessCard() {
  const queryClient = useQueryClient();
  const token = useQuery({ queryKey: API_TOKEN, queryFn: api.apiToken });
  const [shown, setShown] = useState(false);
  const [confirming, setConfirming] = useState(false);
  const rotate = useMutation({
    mutationFn: api.rotateApiToken,
    onSuccess: (fresh) => {
      queryClient.setQueryData(API_TOKEN, fresh);
      // A new key is made to be copied somewhere straight away.
      setShown(true);
      setConfirming(false);
    },
  });

  return (
    <Card>
      <CardHeader>
        <CardTitle>API access</CardTitle>
        <CardDescription>
          Lets scripts, other systems and the command line use Niles:
          everything a signed-in person can do, so treat it like a password.
        </CardDescription>
      </CardHeader>
      <CardContent className="flex flex-col gap-5">
        {token.isPending ? (
          <Skeleton className="h-10 w-full" />
        ) : token.isError ? (
          <p className="text-destructive text-sm">{token.error.message}</p>
        ) : (
          <TheKey
            report={token.data}
            shown={shown}
            onShow={setShown}
            busy={rotate.isPending}
            onMake={() => rotate.mutate()}
            onNew={() => setConfirming(true)}
          />
        )}
        {rotate.error && !confirming && (
          <p className="text-destructive text-sm">{rotate.error.message}</p>
        )}
        <section className="flex flex-col gap-2">
          <h3 className="text-muted-foreground px-1 text-xs font-medium tracking-wide uppercase">
            How to use it
          </h3>
          <p className="text-muted-foreground text-sm">
            Send it as a bearer token with each request. For example, to list
            the devices:
          </p>
          <pre className="bg-muted overflow-x-auto rounded-md px-3 py-2 font-mono text-xs">
            {`curl -H "Authorization: Bearer <token>" ${window.location.origin}/devices`}
          </pre>
        </section>
      </CardContent>
      <ConfirmNewKey
        open={confirming}
        busy={rotate.isPending}
        error={rotate.error?.message}
        onConfirm={() => rotate.mutate()}
        onClose={() => {
          rotate.reset();
          setConfirming(false);
        }}
      />
    </Card>
  );
}

function TheKey({
  report,
  shown,
  onShow,
  busy,
  onMake,
  onNew,
}: {
  report: ApiToken;
  shown: boolean;
  onShow: (shown: boolean) => void;
  busy: boolean;
  onMake: () => void;
  onNew: () => void;
}) {
  if (report.source === "environment") {
    return (
      <p className="text-muted-foreground text-sm">
        Set by the environment variable{" "}
        <code className="font-mono">{report.env_var}</code>; change it there.
      </p>
    );
  }
  if (report.source === "none" || !report.token) {
    return report.writable ? (
      <div className="flex items-center justify-between gap-3">
        <p className="text-muted-foreground text-sm">There is no key yet.</p>
        <Button variant="outline" disabled={busy} onClick={onMake}>
          Make a key
        </Button>
      </div>
    ) : (
      <p className="text-muted-foreground text-sm">
        Niles needs a database to make and keep a key. Without one, a key
        can only be set by the deployment, in an environment variable.
      </p>
    );
  }
  return <KeyRow token={report.token} shown={shown} onShow={onShow} onNew={onNew} />;
}

function KeyRow({
  token,
  shown,
  onShow,
  onNew,
}: {
  token: string;
  shown: boolean;
  onShow: (shown: boolean) => void;
  onNew: () => void;
}) {
  const [copied, setCopied] = useState(false);
  const [copyFailed, setCopyFailed] = useState(false);
  useEffect(() => {
    if (!copied) return;
    const timer = setTimeout(() => setCopied(false), 2000);
    return () => clearTimeout(timer);
  }, [copied]);

  const copy = async () => {
    setCopyFailed(false);
    try {
      await navigator.clipboard.writeText(token);
      setCopied(true);
    } catch {
      // No clipboard outside a secure context — Niles reached over
      // plain HTTP on the house network. Showing it lets it be
      // selected and copied by hand instead.
      setCopyFailed(true);
      onShow(true);
    }
  };

  return (
    <div className="flex flex-col gap-2">
      <div className="flex items-center gap-2">
        <code
          aria-label="API key"
          className="bg-muted min-w-0 flex-1 rounded-md px-3 py-2 font-mono text-xs break-all select-all"
        >
          {shown ? token : "•".repeat(32)}
        </code>
        <Button
          variant="ghost"
          size="icon"
          aria-label={shown ? "Hide the key" : "Show the key"}
          aria-pressed={shown}
          onClick={() => onShow(!shown)}
        >
          {shown ? <EyeOff /> : <Eye />}
        </Button>
        <Button variant="ghost" size="icon" aria-label="Copy the key" onClick={copy}>
          {copied ? <Check /> : <Copy />}
        </Button>
      </div>
      <div className="flex items-center justify-between gap-3">
        <span className="text-muted-foreground text-xs" role="status">
          {copied ? "Copied" : copyFailed ? "Could not copy; select it and copy it by hand" : ""}
        </span>
        <Button variant="outline" size="sm" onClick={onNew}>
          New key
        </Button>
      </div>
    </div>
  );
}

function ConfirmNewKey({
  open,
  busy,
  error,
  onConfirm,
  onClose,
}: {
  open: boolean;
  busy: boolean;
  error?: string;
  onConfirm: () => void;
  onClose: () => void;
}) {
  return (
    <Dialog
      open={open}
      onOpenChange={(next: boolean) => {
        if (!next) onClose();
      }}
    >
      <DialogContent className="gap-0 p-0">
        <div className="border-border border-b px-4 py-3">
          <DialogTitle>Make a new key?</DialogTitle>
          <DialogDescription>
            Scripts using the old key will stop working.
          </DialogDescription>
        </div>
        <DialogBody className="flex flex-col gap-3">
          {error && <p className="text-destructive text-sm">{error}</p>}
          <div className="flex justify-end gap-2">
            <Button variant="outline" onClick={onClose}>
              Keep this one
            </Button>
            <Button variant="destructive" disabled={busy} onClick={onConfirm}>
              Make a new key
            </Button>
          </div>
        </DialogBody>
      </DialogContent>
    </Dialog>
  );
}
