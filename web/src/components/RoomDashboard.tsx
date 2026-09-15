import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { AlertTriangle } from "lucide-react";
import { RoomCard } from "@/components/RoomCard";
import type { SetZone } from "@/components/RoomCard";
import { Skeleton } from "@/components/ui/skeleton";
import { useDeviceStream } from "@/hooks/useDeviceStream";
import { ApiError, api } from "@/lib/api";
import type { Device, SetLight } from "@/lib/api";
import { HouseBar } from "@/components/HouseBar";
import { SceneBar } from "@/components/SceneBar";
import { houseToggle, optimistic, roomsOf, targets } from "@/lib/rooms";
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

  const scenes = useQuery({ queryKey: ["scenes"], queryFn: api.scenes });
  // Each call reaches tado, so this is polled slowly. Heating moves in
  // tens of minutes; a radiator is not a light switch.
  const climate = useQuery({
    queryKey: ["climate"],
    queryFn: api.climate,
    refetchInterval: 120_000,
  });
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

  const rooms = roomsOf(devices.data ?? [], climate.data ?? []);
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
      <HouseBar
        rooms={rooms}
        onToggle={() =>
          command.mutate({ target: { scope: "house" }, body: houseToggle(rooms) })
        }
      />
      <div className="grid grid-cols-2 gap-3 lg:grid-cols-3">
        {rooms.map((room) => (
          <RoomCard
            key={room.name}
            room={room}
            onSetRoom={(body) =>
              command.mutate({ target: { scope: "room", room: room.name }, body })
            }
            onSetLight={(light, body) =>
              command.mutate({ target: { scope: "light", light }, body })
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

function LoadingGrid() {
  return (
    <div className="grid grid-cols-2 gap-3 lg:grid-cols-3">
      {[0, 1, 2, 3, 4, 5].map((i) => (
        <Skeleton key={i} className="h-28 w-full rounded-xl" />
      ))}
    </div>
  );
}
