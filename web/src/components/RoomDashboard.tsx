import { useEffect, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { AlertTriangle, EyeOff } from "lucide-react";
import { RoomCard } from "@/components/RoomCard";
import type { SetZone } from "@/components/RoomCard";
import { Skeleton } from "@/components/ui/skeleton";
import { useDeviceStream } from "@/hooks/useDeviceStream";
import { ApiError, api, roomOrder, roomsHidden } from "@/lib/api";
import type { BlindsOpened, Device, SetLight } from "@/lib/api";
import { useTvVolume } from "@/hooks/useTvVolume";
import { BoostButton } from "@/components/BoostButton";
import { GroceryDeliveryCard } from "@/components/GroceryDeliveryCard";
import { HouseBar } from "@/components/HouseBar";
import { PairPhoneCard } from "@/components/PairPhoneCard";
import { PluginWidgetCard } from "@/components/PluginWidgets";
import { placeWidgets } from "@/lib/plugins";
import { SceneBar } from "@/components/SceneBar";
import { TodaysRunCard } from "@/components/TodaysRunCard";
import {
  boostEndsAt,
  boosted,
  houseToggle,
  onTheGrid,
  optimistic,
  roomsOf,
  targets,
  withPosition,
} from "@/lib/rooms";
import type { Target } from "@/lib/rooms";

/**
 * Every room in the house, and its lights.
 *
 * Built from the registry rather than configured: a light Niles has
 * discovered shows up here without anyone naming it twice, and a room
 * exists because something in it does. There is nothing to keep in
 * step, which is the point.
 */
export function RoomDashboard() {
  const queryClient = useQueryClient();
  const [error, setError] = useState<string | null>(null);
  useDeviceStream();

  // Asked once per visit and never retried: a 501 means there is no
  // console, which is a fine answer, and the card renders nothing until
  // an answer exists — so it can never appear and then withdraw.
  const phone = useQuery({
    queryKey: ["presence-device"],
    queryFn: api.deviceStatus,
    retry: false,
    staleTime: Infinity,
  });
  const pair = useMutation({
    mutationFn: api.pairDevice,
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["presence-device"] }),
  });

  const devices = useQuery({
    queryKey: ["devices"],
    queryFn: api.devices,
    // The stream keeps this fresh, so refetching is only a safety net
    // for a socket that never connected.
    refetchInterval: 60_000,
  });

  const command = useMutation({
    mutationFn: ({ target, body }: { target: Target; body: SetLight }) => {
      switch (target.scope) {
        case "house":
          return api.setAllLights(body).then(() => undefined);
        case "room":
          return api.setRoom(target.room, body).then(() => undefined);
        case "light":
          return api.setLight(target.light, body).then(() => undefined);
      }
    },
    // Show the press landing. The light's own report arrives over the
    // stream a moment later and replaces this with the truth; if the
    // command failed, the rollback puts it back.
    onMutate: async ({ target, body }) => {
      setError(null);
      await queryClient.cancelQueries({ queryKey: ["devices"] });
      const previous = queryClient.getQueryData<Device[]>(["devices"]);
      queryClient.setQueryData<Device[]>(["devices"], (current) =>
        current?.map((device) =>
          targets(device, target) ? optimistic(device, body) : device,
        ),
      );
      return { previous };
    },
    onError: (failure, _variables, context) => {
      if (context?.previous) {
        queryClient.setQueryData(["devices"], context.previous);
      }
      setError(
        failure instanceof ApiError ? failure.message : String(failure),
      );
    },
  });

  // One request per blind for a position; one request for the whole
  // set for an Open, because where a second press goes is a property
  // of the set and only the server resolves it.
  const blinds = useMutation({
    mutationFn: ({
      blinds,
      position,
    }: {
      blinds: Device[];
      position: number | "open";
    }): Promise<BlindsOpened | undefined> =>
      position === "open"
        ? api.openBlinds(blinds.map((blind) => blind.id))
        : Promise.all(
            blinds.map((blind) => api.setPosition(blind.room, blind.name, position)),
          ).then(() => undefined),
    onMutate: async ({ blinds, position }) => {
      setError(null);
      await queryClient.cancelQueries({ queryKey: ["devices"] });
      const previous = queryClient.getQueryData<Device[]>(["devices"]);
      const moving = new Set(blinds.map((blind) => blind.id));
      // An Open has no position to guess — guessing 100 would flash
      // the wrong number before settling on 75 — so the row waits for
      // the answer and onSuccess paints it.
      if (typeof position === "number") {
        queryClient.setQueryData<Device[]>(["devices"], (current) =>
          current?.map((device) =>
            moving.has(device.id) ? withPosition(device, position) : device,
          ),
        );
      }
      return { previous };
    },
    onSuccess: (opened) => {
      // An Open's answer, painted where a number would have been
      // guessed. No refetch: the blind reports its position over the
      // event stream as it moves.
      if (!opened) return;
      queryClient.setQueryData<Device[]>(["devices"], (current) =>
        current?.map((device) => {
          const sent = opened.positions.find((p) => p.id === device.id);
          return sent ? withPosition(device, sent.position) : device;
        }),
      );
    },
    onError: (failure, _variables, context) => {
      if (context?.previous) {
        queryClient.setQueryData(["devices"], context.previous);
      }
      setError(
        failure instanceof ApiError ? failure.message : String(failure),
      );
    },
  });

  const scenes = useQuery({ queryKey: ["scenes"], queryFn: api.scenes });
  // Answers an error when nemlig.com is not switched on, and then there
  // is simply no card. Orders do not move often.
  const delivery = useQuery({
    queryKey: ["nemlig-next"],
    queryFn: api.nemligNext,
    retry: false,
    staleTime: 15 * 60_000,
  });
  // Asks the TV itself, so not often; a press refreshes it at once.
  const tv = useQuery({
    queryKey: ["tv"],
    queryFn: api.tv,
    retry: false,
    refetchInterval: 60_000,
  });
  const tvPower = useMutation({
    mutationFn: api.tvPower,
    // A wake-up the TV did not answer says where to look.
    onSuccess: (powered) => setError(powered?.message ?? null),
    onError: (failure) =>
      setError(failure instanceof ApiError ? failure.message : String(failure)),
    // A TV takes a few seconds to come on; ask again once it has.
    onSettled: () =>
      setTimeout(() => queryClient.invalidateQueries({ queryKey: ["tv"] }), 4_000),
  });
  const tvSound = useTvVolume({
    onSuccess: () => setError(null),
    onError: (failure) =>
      setError(failure instanceof ApiError ? failure.message : String(failure)),
  });
  // What each room's Sonos plays. Asks the speakers, so not every
  // second; a press refreshes it at once.
  const music = useQuery({
    queryKey: ["music"],
    queryFn: api.music,
    retry: false,
    refetchInterval: 15_000,
  });
  // Each call reaches tado, so this is polled slowly. Heating moves in
  // tens of minutes; a radiator is not a light switch.
  const climate = useQuery({
    queryKey: ["climate"],
    queryFn: api.climate,
    refetchInterval: 120_000,
  });
  const boost = useMutation({
    mutationFn: (running: number[]) =>
      running.length > 0
        ? api.resumeZones(running).then(() => undefined)
        : api.boostHeating().then(() => undefined),
    onError: (failure) =>
      setError(
        failure instanceof ApiError
          ? failure.message
          : "Couldn't reach Niles to change the heating.",
      ),
    // Every room's target has just changed, and this is the one page
    // that shows them all.
    onSuccess: () => {
      setError(null);
      queryClient.invalidateQueries({ queryKey: ["climate"] });
    },
  });


  // Only for the order the cards sit in and which are hidden, which is
  // why nothing here waits on it: rooms in the alphabet for the
  // half-second before it lands is a page that arranges itself, not a
  // page that is missing something. The key is the one Settings writes
  // and invalidates.
  const config = useQuery({ queryKey: ["config"], queryFn: api.getConfig });
  // For the plugins' widgets. A 501 is a Niles without plugins: no
  // widgets, and nothing to ask again.
  const plugins = useQuery({ queryKey: ["plugins"], queryFn: api.plugins, retry: false });
  const setZone = useMutation({
    mutationFn: ({ zone, body }: { zone: number; body: SetZone }) =>
      api.setZone(zone, body),
    onError: (failure) =>
      setError(failure instanceof ApiError ? failure.message : String(failure)),
    // tado takes a moment to report the change back, and asking
    // immediately would show the old answer as if the press had missed.
    onSuccess: () =>
      setTimeout(
        () => queryClient.invalidateQueries({ queryKey: ["climate"] }),
        1500,
      ),
  });
  const applyScene = useMutation({
    mutationFn: (name: string) => api.applyScene(name),
    // A scene moves several lights at once, and the reports arrive
    // over the event stream; asking again closes the gap for anything
    // that does not report.
    onSettled: () => queryClient.invalidateQueries({ queryKey: ["devices"] }),
  });

  /**
   * Ask again the moment a boost runs out.
   *
   * Tado ends it on its own clock and tells nobody, so without this the
   * button goes on offering to end a boost that is already over until
   * the next poll — up to two minutes of a control that does nothing.
   * A second of slack, because the two clocks are not the same one.
   */
  const endsAt = boostEndsAt(climate.data ?? []);
  useEffect(() => {
    if (endsAt === null) return;
    const wait = Math.max(0, endsAt - Date.now()) + 1_000;
    const timer = setTimeout(
      () => queryClient.invalidateQueries({ queryKey: ["climate"] }),
      wait,
    );
    return () => clearTimeout(timer);
  }, [endsAt, queryClient]);

  // Every hook is above the early returns. React counts them per
  // render, so one sitting below `isLoading` is called on the second
  // render and not the first — which is not a warning, it is the whole
  // page unmounting the moment the devices arrive.
  if (devices.isLoading) return <LoadingGrid />;
  if (devices.error) {
    return (
      <Notice>Can't reach Niles: {String(devices.error)}</Notice>
    );
  }

  const rooms = roomsOf(
    devices.data ?? [],
    climate.data ?? [],
    roomOrder(config.data?.effective),
  );
  // What a boost has left running. Derived rather than remembered: a
  // boost started on a phone is one this page can end, and one that
  // expired while nobody was looking is one it has already forgotten.
  const running = boosted(climate.data ?? []);
  // Every room, hidden or not: a widget placed in a hidden room stays
  // placed there rather than turning up on the front page instead.
  const widgets = placeWidgets(
    plugins.data ?? [],
    config.data?.effective,
    rooms.map((room) => room.name),
  );
  const grid = onTheGrid(rooms, roomsHidden(config.data?.effective));
  if (rooms.length === 0) {
    return (
      <Notice>
        Niles hasn't discovered any lights yet. Rooms appear here as soon as
        it does — there is nothing to configure.
      </Notice>
    );
  }

  return (
    <div className="flex flex-col gap-3">
      {error && <Notice>{error}</Notice>}
      <SceneBar
        scenes={scenes.data ?? []}
        onApply={(name) => applyScene.mutate(name)}
      />
      {/* The two statements about the whole house, side by side. Boost
          only where there is heating to boost: a button that reports
          "no tado connection" is a button that should not be there. */}
      <div className="flex items-stretch gap-2">
        {rooms.some((room) => room.zone) && (
          <BoostButton
            boosting={running.length > 0}
            pending={boost.isPending}
            onBoost={() => boost.mutate([])}
            onResume={() => boost.mutate(running)}
          />
        )}
        <div className="min-w-0 flex-1">
          <HouseBar
            rooms={rooms}
            onToggle={() =>
              command.mutate({
                target: { scope: "house" },
                body: houseToggle(rooms),
              })
            }
          />
        </div>
      </div>
      <TodaysRunCard />
      <GroceryDeliveryCard
        order={delivery.data}
        // Local today: the order's times are Danish, and so is the house.
        today={new Date().toLocaleDateString("sv-SE")}
      />
      <PairPhoneCard
        device={phone.data}
        pairing={pair.isPending}
        onPair={() => pair.mutate()}
      />
      {widgets.front.length > 0 && (
        <div className="grid gap-3 sm:grid-cols-2">
          {widgets.front.map((placed) => (
            <PluginWidgetCard key={`${placed.plugin}.${placed.widget.id}`} placed={placed} />
          ))}
        </div>
      )}
      {grid.length === 0 && <AllHidden />}
      <div className="grid grid-cols-2 gap-3 lg:grid-cols-3">
        {grid.map((room) => (
          <RoomCard
            key={room.name}
            room={room}
            music={music.data?.find((m) => m.room === room.name)}
            widgets={widgets.rooms[room.name]}
            tv={
              tv.data?.paired && tv.data.room === room.name
                ? {
                    info: tv.data,
                    busy: tvPower.isPending,
                    onPower: (on) => tvPower.mutate(on),
                    // Until the speakers have answered, a room with a
                    // Sonos cannot be told from one without.
                    onVolume: music.isPending
                      ? undefined
                      : (change) => tvSound.mutate(change),
                  }
                : undefined
            }
            onSetRoom={(body) =>
              command.mutate({ target: { scope: "room", room: room.name }, body })
            }
            onSetLight={(light, body) =>
              command.mutate({ target: { scope: "light", light }, body })
            }
            onSetBlinds={(chosen, position) =>
              blinds.mutate({ blinds: chosen, position })
            }
            onSetZone={
              room.zone
                ? (body) => setZone.mutate({ zone: room.zone!.id, body })
                : undefined
            }
          />
        ))}
      </div>
    </div>
  );
}

function Notice({ children }: { children: React.ReactNode }) {
  return (
    <div className="bg-muted/60 flex items-center gap-2 rounded-md px-3 py-2 text-sm">
      <AlertTriangle className="size-4 shrink-0" />
      <span>{children}</span>
    </div>
  );
}

/**
 * Every room hidden. Not a warning: it is what somebody asked for, and
 * the page only has to say where the rooms went and how to get them
 * back.
 */
function AllHidden() {
  return (
    <div className="text-muted-foreground flex flex-col items-center gap-2 rounded-xl border border-dashed px-4 py-10 text-center text-sm">
      <EyeOff aria-hidden className="size-5" />
      <p>Every room is hidden from the dashboard.</p>
      <a
        href="#/me/settings/rooms"
        className="text-foreground underline underline-offset-4"
      >
        Show them in Settings → Rooms
      </a>
    </div>
  );
}

function LoadingGrid() {
  return (
    <div className="grid grid-cols-2 gap-3 lg:grid-cols-3">
      {[0, 1, 2, 3, 4, 5].map((i) => (
        <Skeleton key={i} className="h-28 w-full rounded-xl" />
      ))}
    </div>
  );
}
