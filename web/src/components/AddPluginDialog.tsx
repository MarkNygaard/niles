import { useCallback, useEffect, useRef, useState, type FormEvent } from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Dialog,
  DialogBody,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { PluginPreviewView } from "@/components/PluginPreview";
import { api } from "@/lib/api";
import type { PluginPreview, PluginSummary } from "@/lib/api";

export interface AddPluginDialogProps {
  open: boolean;
  /** An address to preview as soon as it opens: Browse's Install. */
  address?: string;
  onOpenChange: (open: boolean) => void;
  onInstalled: (plugin: PluginSummary) => void;
}

/**
 * Add-on from GitHub: a repo's address, then what its latest release would
 * do, then Install.
 *
 * Two requests, so what is installed is what was on the screen: the
 * preview downloads and checks the release and keeps it under a token,
 * and Install hands back the token and the checksum it showed.
 */
export function AddPluginDialog({ open, address, onOpenChange, onInstalled }: AddPluginDialogProps) {
  const [repo, setRepo] = useState("");
  const [preview, setPreview] = useState<PluginPreview | null>(null);
  const [busy, setBusy] = useState<"preview" | "install" | null>(null);
  const [error, setError] = useState<string | null>(null);
  // Each opening is its own: a preview still on its way when the dialog
  // closes must not land in the next one.
  const opening = useRef(0);
  const started = useRef<string | null>(null);

  function close() {
    opening.current += 1;
    started.current = null;
    setRepo("");
    setPreview(null);
    setBusy(null);
    setError(null);
    onOpenChange(false);
  }

  const lookUp = useCallback(async (address: string) => {
    const mine = opening.current;
    setBusy("preview");
    setError(null);
    try {
      const found = await api.previewPlugin(address);
      if (mine === opening.current) setPreview(found);
    } catch (e) {
      if (mine === opening.current) setError(e instanceof Error ? e.message : String(e));
    } finally {
      if (mine === opening.current) setBusy(null);
    }
  }, []);

  function look(event: FormEvent) {
    event.preventDefault();
    void lookUp(repo.trim());
  }

  useEffect(() => {
    if (!open || !address || started.current === address) return;
    started.current = address;
    setRepo(address);
    void lookUp(address);
  }, [open, address, lookUp]);

  async function install() {
    if (!preview) return;
    setBusy("install");
    setError(null);
    try {
      const installed = await api.installPlugin(preview);
      close();
      onInstalled(installed);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  }

  return (
    <Dialog open={open} onOpenChange={(next: boolean) => (next ? onOpenChange(true) : close())}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <div className="flex min-w-0 flex-col gap-1">
            <DialogTitle>Add-on from GitHub</DialogTitle>
            <DialogDescription>
              A third-party add-on somebody published as a release on GitHub. It runs walled off
              from the rest of Niles, with only what you agree to here.
            </DialogDescription>
          </div>
        </DialogHeader>
        <DialogBody className="flex flex-col gap-4">
          {preview ? (
            <>
              <PluginPreviewView preview={preview} />
              <div className="flex gap-2">
                <Button disabled={busy !== null} onClick={install}>
                  {busy === "install" ? "Installing…" : "Install"}
                </Button>
                <Button variant="ghost" disabled={busy !== null} onClick={close}>
                  Cancel
                </Button>
              </div>
            </>
          ) : (
            <form onSubmit={look} className="flex flex-col gap-2">
              <div className="flex gap-2">
                <Input
                  aria-label="GitHub repository"
                  placeholder="https://github.com/jane/niles-spotprice"
                  autoComplete="off"
                  value={repo}
                  onChange={(e) => setRepo(e.target.value)}
                />
                <Button type="submit" disabled={busy !== null || !repo.trim()}>
                  {busy === "preview" ? "Looking…" : "Preview"}
                </Button>
              </div>
              <p className="text-muted-foreground text-xs">
                Its latest release is checked and shown before anything is installed.
              </p>
            </form>
          )}
          {error && (
            <p role="alert" className="text-destructive text-sm">
              {error}
            </p>
          )}
        </DialogBody>
      </DialogContent>
    </Dialog>
  );
}
