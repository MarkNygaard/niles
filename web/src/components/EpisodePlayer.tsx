import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Skeleton } from "@/components/ui/skeleton";
import { api } from "@/lib/api";
import { dayLabel } from "@/lib/trainingPlan";

/**
 * A recorded episode's player: the signed address the episode is
 * played from, fetched through Niles so no bucket address or key ever
 * reaches the page, and how long the episode is kept.
 *
 * The address is signed for four hours, so a pause longer than that
 * fails the element once: it is asked again, and only then does the
 * player say the recording could not be loaded.
 */
export function EpisodePlayer({ artifact, title }: { artifact: number; title: string }) {
  const [askedAgain, setAskedAgain] = useState(false);
  const [loadFailed, setLoadFailed] = useState(false);
  const audio = useQuery({
    // Artifact metadata changes must not replace the source during playback.
    queryKey: ["episode-audio", artifact],
    queryFn: () => api.episodeAudio(artifact),
    staleTime: 60 * 60 * 1000,
    refetchOnWindowFocus: false,
    refetchOnReconnect: false,
  });

  if (audio.isPending) return <Skeleton className="h-10 w-full" />;
  if (audio.isError) {
    return (
      <p className="text-destructive text-sm" role="status">
        {audio.error.message}
      </p>
    );
  }

  const { url, kept_until } = audio.data;

  return (
    <div className="flex flex-col gap-1.5">
      {loadFailed ? (
        <p className="text-destructive text-sm" role="status">
          The recording couldn't be loaded.
        </p>
      ) : (
        <audio
          controls
          preload="metadata"
          src={url}
          aria-label={`Play ${title}`}
          className="w-full"
          onError={() => {
            if (askedAgain) {
              setLoadFailed(true);
              return;
            }
            setAskedAgain(true);
            void audio.refetch().then((result) => {
              // Refetched within the same second, the signed address is
              // byte-identical and would fail the same way: said, not
              // retried against itself.
              if (result.data?.url === url) setLoadFailed(true);
            });
          }}
        />
      )}
      <p className="text-muted-foreground text-xs">
        Kept until {dayLabel(kept_until.slice(0, 10))}
      </p>
      <p className="text-muted-foreground text-xs">
        To play it on a Sonos, ask Niles: “play it in the kitchen”.
      </p>
    </div>
  );
}
