import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api";
import { RESTART } from "@/lib/restart";
import { useDeviceStream } from "@/hooks/useDeviceStream";

/** What waits for a restart, kept current by the events stream. */
export function useRestartStatus(enabled = true) {
  useDeviceStream(enabled);
  return useQuery({
    queryKey: RESTART,
    queryFn: api.restartStatus,
    enabled,
    retry: false,
  });
}
