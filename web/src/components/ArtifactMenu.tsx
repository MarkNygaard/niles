import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Check, Copy, FileDown, FileText, LoaderCircle, Podcast, Share, Users } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { api } from "@/lib/api";
import type { ArtifactDetail } from "@/lib/api";
import { kindOf, markdownOf } from "@/lib/artifacts";
import { WORD_KINDS, fileName, saveFile } from "@/lib/downloads";

export interface ArtifactMenuProps {
  artifact: ArtifactDetail;
  /** What is shown: the version being read, or the current one.
      Undefined while it loads, null before there is any. */
  content: unknown;
  /** The version being read in place of the current one. */
  reading?: number;
  /** Its owner's alone. */
  onShare?: () => void;
}

/**
 * The share symbol in an open artifact's corner, and what is behind it:
 * sharing it, copying it, and taking it away as a file. A file is made
 * from what is shown, so an old version read gives that version, named
 * so; and from the content, never from the artifact's page.
 */
export function ArtifactMenu({ artifact: a, content, reading, onShare }: ArtifactMenuProps) {
  const [open, setOpen] = useState(false);
  const [copied, setCopied] = useState(false);
  const [making, setMaking] = useState(false);
  const [failed, setFailed] = useState<string>();
  const kind = kindOf(a.kind);
  const shown = content !== undefined && content !== null;
  const episode = a.kind === "podcast" && a.status === "ready" && reading === undefined;
  // The player's own query, so the menu asks for no second address.
  const audio = useQuery({
    queryKey: ["episode-audio", a.id],
    queryFn: () => api.episodeAudio(a.id),
    enabled: episode,
    staleTime: 60 * 60 * 1000,
    refetchOnWindowFocus: false,
    refetchOnReconnect: false,
  });
  const episodeUrl = episode ? audio.data?.url : undefined;
  const word = shown && WORD_KINDS.has(a.kind);
  const markdown = shown && a.kind === "document";

  if (!onShare && !shown && !episodeUrl) return null;

  const downloadWord = async () => {
    setMaking(true);
    setFailed(undefined);
    try {
      const { wordFile } = await import("@/lib/wordExport");
      const blob = await wordFile({
        kind: a.kind,
        title: a.title,
        content,
        images: a.images,
        loadImage: (image) => api.artifactImage(a.id, image),
      });
      saveFile(blob, fileName(a.title, "docx", reading));
      setOpen(false);
    } catch (e) {
      setFailed(`The Word file could not be made: ${e instanceof Error ? e.message : String(e)}`);
    } finally {
      setMaking(false);
    }
  };

  return (
    <DropdownMenu
      open={open}
      onOpenChange={(next) => {
        setOpen(next);
        if (next) {
          setCopied(false);
          setFailed(undefined);
        }
      }}
    >
      <DropdownMenuTrigger
        render={<Button variant="ghost" size="icon-sm" className="shrink-0" />}
        aria-label="Share and download"
      >
        <Share />
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-56">
        {onShare && (
          <DropdownMenuItem onClick={onShare}>
            <Users />
            Share…
          </DropdownMenuItem>
        )}
        {shown && (
          <DropdownMenuItem
            closeOnClick={false}
            onClick={async () => {
              await navigator.clipboard?.writeText(kind.text(content));
              setCopied(true);
            }}
          >
            {copied ? <Check /> : <Copy />}
            {copied ? "Copied" : "Copy"}
          </DropdownMenuItem>
        )}
        {word && (
          <DropdownMenuItem closeOnClick={false} disabled={making} onClick={downloadWord}>
            {making ? <LoaderCircle className="animate-spin" /> : <FileDown />}
            {making ? "Making the Word file…" : "Download as Word"}
          </DropdownMenuItem>
        )}
        {markdown && (
          <DropdownMenuItem
            onClick={() =>
              saveFile(
                new Blob([markdownOf(content)], {
                  type: "text/markdown;charset=utf-8",
                }),
                fileName(a.title, "md", reading),
              )
            }
          >
            <FileText />
            Download as Markdown
          </DropdownMenuItem>
        )}
        {episodeUrl && (
          // The address is the bucket's, another site's, so a browser
          // opens the episode in a tab rather than saving it straight
          // away; it is saved from there.
          <DropdownMenuItem
            render={
              <a
                href={episodeUrl}
                download={fileName(a.title, "mp3")}
                target="_blank"
                rel="noopener noreferrer"
              />
            }
          >
            <Podcast />
            Download the episode
          </DropdownMenuItem>
        )}
        {failed && (
          <p role="status" className="text-destructive px-1.5 py-1 text-xs">
            {failed}
          </p>
        )}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
