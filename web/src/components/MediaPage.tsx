import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Skeleton } from "@/components/ui/skeleton";
import { MusicRow } from "@/components/MusicRow";
import { TvRow } from "@/components/TvRow";
import { api } from "@/lib/api";
import type { MusicControl, RoomMusic } from "@/lib/api";
import { humanize } from "@/lib/rooms";

/** Rooms that play first, then the rest by name. */
export function mediaOrder(rooms: RoomMusic[]): RoomMusic[] {
  return [...rooms].sort(
    (a, b) => Number(b.playing) - Number(a.playing) || a.room.localeCompare(b.room),
  );
}

/** Whether anything plays — music, radio, or the TV on — for the menu's
    "when something is playing". */
export function somethingPlays(
  music?: RoomMusic[],
  tvOn?: boolean,
): boolean {
  return Boolean(tvOn) || Boolean(music?.some((m) => m.playing));
}

/**
 * Everything that plays, in one place: the TV, and each room's Sonos
 * with what it plays, pause or play, and its volume. The room cards
 * only mark that something plays; this is where it is turned up.
 */
export function MediaPage() {
  const queryClient = useQueryClient();
  const music = useQuery({
    queryKey: ["music"],
    queryFn: api.music,
    retry: false,
    refetchInterval: 10_000,
  });
  const tv = useQuery({
    queryKey: ["tv"],
    queryFn: api.tv,
    retry: false,
    refetchInterval: 30_000,
  });
  const control = useMutation({
    mutationFn: ({ room, body }: { room: string; body: MusicControl }) =>
      api.musicControl(room, body),
    onSettled: () => queryClient.invalidateQueries({ queryKey: ["music"] }),
  });
  const tvPower = useMutation({
    mutationFn: api.tvPower,
    // A TV takes a few seconds to come on; ask again once it has.
    onSettled: () =>
      setTimeout(() => queryClient.invalidateQueries({ queryKey: ["tv"] }), 4_000),
  });

  if (music.isLoading) {
    return (
      <div className="flex flex-col gap-3">
        <Skeleton className="h-16 w-full" />
        <Skeleton className="h-24 w-full" />
      </div>
    );
  }

  const rooms = mediaOrder(music.data ?? []);
  const pairedTv = tv.data?.paired ? tv.data : undefined;

  return (
    <div className="flex flex-col gap-3">
      {pairedTv && (
        <div className="bg-card rounded-xl px-4">
          <TvRow
            tv={pairedTv}
            busy={tvPower.isPending}
            onPower={(on) => tvPower.mutate(on)}
          />
        </div>
      )}
      {rooms.map((room) => (
        <div key={room.room} className="bg-card rounded-xl px-4">
          <MusicRow
            title={humanize(room.room)}
            music={room}
            busy={control.isPending}
            onPause={() => control.mutate({ room: room.room, body: { action: "pause" } })}
            onPlay={() => control.mutate({ room: room.room, body: { action: "play" } })}
            onVolume={(percent) =>
              control.mutate({ room: room.room, body: { action: "volume", percent } })
            }
          />
        </div>
      ))}
      {rooms.length === 0 && !pairedTv && (
        <p className="text-muted-foreground text-sm">
          No speakers or TV are set up. Add Sonos or an LG TV under Settings →
          Integrations.
        </p>
      )}
    </div>
  );
}
