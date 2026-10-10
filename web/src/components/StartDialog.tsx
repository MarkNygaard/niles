import { useState } from "react";
import { useMutation, useQuery } from "@tanstack/react-query";
import { Check, Loader2, Search, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogBody,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { api } from "@/lib/api";
import type { MediaChoice, MediaSpeaker } from "@/lib/api";
import { humanize } from "@/lib/rooms";
import { cn } from "@/lib/utils";

export interface StartDialogProps {
  /** What to start, or nothing when the dialog is closed. */
  source?: "radio" | "spotify";
  /** The speakers ticked to begin with: the one dropped on the card. */
  preset: string[];
  speakers: MediaSpeaker[];
  onClose: () => void;
  onStarted: () => void;
}

/** The headings the choices fall under, in the order they are shown. */
export function choiceSections(choices: MediaChoice[]): { title: string; choices: MediaChoice[] }[] {
  const titles: Record<MediaChoice["kind"], string> = {
    queue: "Carry on",
    favorite: "Favorites",
    remembered: "Played last",
    station: "Stations",
    spotify: "On Spotify",
  };
  const order: MediaChoice["kind"][] = ["queue", "favorite", "remembered", "station", "spotify"];
  return order
    .map((kind) => ({ title: titles[kind], choices: choices.filter((c) => c.kind === kind) }))
    .filter((section) => section.choices.length > 0);
}

/**
 * Something new on the radio or Spotify: which speakers, and what.
 *
 * Without a search it offers what is close to hand — the radio's
 * favorite stations and the ones rooms played last; Spotify's queues a
 * speaker can carry on and the playlists among the favorites. A search
 * asks TuneIn or Spotify.
 */
export function StartDialog({ source, preset, speakers, onClose, onStarted }: StartDialogProps) {
  return (
    <Dialog
      open={source !== undefined}
      onOpenChange={(open: boolean) => {
        if (!open) onClose();
      }}
    >
      <DialogContent
        initialFocus={(openType) => openType === "keyboard"}
        className="flex h-[85dvh] flex-col p-0 sm:h-[min(80vh,40rem)]"
      >
        {source && (
          <Starter
            key={`${source}:${preset.join(",")}`}
            source={source}
            preset={preset}
            speakers={speakers}
            onStarted={() => {
              onStarted();
              onClose();
            }}
          />
        )}
      </DialogContent>
    </Dialog>
  );
}

function Starter({
  source,
  preset,
  speakers,
  onStarted,
}: {
  source: "radio" | "spotify";
  preset: string[];
  speakers: MediaSpeaker[];
  onStarted: () => void;
}) {
  const [chosen, setChosen] = useState<Set<string>>(() => new Set(preset));
  const [draft, setDraft] = useState("");
  const [query, setQuery] = useState("");
  const title = source === "radio" ? "the radio" : "Spotify";
  const choices = useQuery({
    queryKey: ["media-choices", source, query],
    queryFn: () => api.mediaChoices(source, query || undefined),
    retry: false,
  });
  const start = useMutation({
    mutationFn: (choice: MediaChoice) => api.mediaStart([...chosen], choice),
    onSuccess: onStarted,
  });
  const toggle = (id: string) =>
    setChosen((now) => {
      const next = new Set(now);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });

  return (
    <>
      <div className="border-border flex items-start gap-3 border-b px-4 py-3">
        <div className="min-w-0 flex-1">
          <DialogTitle>Play {title}</DialogTitle>
          <DialogDescription>Tick the speakers, then pick what to play.</DialogDescription>
        </div>
        <DialogClose
          aria-label="Close"
          className="text-muted-foreground hover:text-foreground focus-visible:ring-3 focus-visible:ring-ring/50 flex size-8 shrink-0 items-center justify-center rounded-lg focus-visible:outline-none"
        >
          <X aria-hidden className="size-4" />
        </DialogClose>
      </div>

      <div className="flex flex-wrap gap-2 px-4 pt-3" role="group" aria-label="Speakers">
        {speakers.map((speaker) => {
          const on = chosen.has(speaker.id);
          return (
            <button
              key={speaker.id}
              type="button"
              role="checkbox"
              aria-checked={on}
              onClick={() => toggle(speaker.id)}
              className={cn(
                "flex items-center gap-2 rounded-full border px-3 py-1.5 text-sm transition-colors",
                "focus-visible:ring-3 focus-visible:ring-ring/50 focus-visible:outline-none",
                on ? "bg-foreground text-background border-foreground" : "hover:bg-muted",
              )}
            >
              <span
                aria-hidden
                className={cn(
                  "flex size-4 items-center justify-center rounded-sm border",
                  on ? "border-background" : "border-muted-foreground",
                )}
              >
                {on && <Check className="size-3" />}
              </span>
              {speaker.name}
              {speaker.room && humanize(speaker.room) !== speaker.name && (
                <span className={on ? "opacity-70" : "text-muted-foreground"}>
                  · {humanize(speaker.room)}
                </span>
              )}
            </button>
          );
        })}
      </div>

      <form
        className="flex gap-2 px-4 pt-3"
        onSubmit={(e) => {
          e.preventDefault();
          setQuery(draft.trim());
        }}
      >
        <Input
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          aria-label={source === "radio" ? "Search TuneIn" : "Search Spotify"}
          placeholder={source === "radio" ? "A station" : "An artist, album, song or playlist"}
          autoCorrect="off"
          className="h-9"
        />
        <Button type="submit" variant="outline" size="icon-lg" aria-label="Search">
          {choices.isFetching ? <Loader2 className="animate-spin" /> : <Search />}
        </Button>
      </form>

      <DialogBody className="flex flex-col gap-3">
        {start.isError && <p className="text-destructive text-sm">{start.error.message}</p>}
        {choices.isError && <p className="text-destructive text-sm">{choices.error.message}</p>}
        {chosen.size === 0 && (
          <p className="text-muted-foreground text-sm">Tick a speaker to play on.</p>
        )}
        {choices.data?.length === 0 && (
          <p className="text-muted-foreground text-sm">
            {query ? `Nothing found for “${query}”.` : "Nothing to hand. Search for something."}
          </p>
        )}
        {choiceSections(choices.data ?? []).map((section) => (
          <div key={section.title} className="flex flex-col gap-0.5">
            <h3 className="text-muted-foreground px-2 text-xs font-medium">{section.title}</h3>
            {section.choices.map((choice) => (
              <Button
                key={`${choice.kind}:${choice.id}`}
                variant="ghost"
                className="h-auto justify-start py-2 text-left whitespace-normal"
                disabled={chosen.size === 0 || start.isPending}
                onClick={() => start.mutate(choice)}
              >
                {start.isPending && start.variables === choice ? (
                  <Loader2 aria-hidden className="animate-spin" />
                ) : null}
                {choice.label}
              </Button>
            ))}
          </div>
        ))}
      </DialogBody>
    </>
  );
}
