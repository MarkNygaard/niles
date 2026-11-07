import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api";
import { UPDATE } from "@/lib/update";
import { useDeviceStream } from "@/hooks/useDeviceStream";

/** Whether a newer Niles is published, kept current by the events
    stream. Never waits on the network: this is the kept answer, and
    `update_checked` says when to ask again. */
export function useUpdateStatus(enabled = true) {
  useDeviceStream(enabled);
  return useQuery({
    queryKey: UPDATE,
    queryFn: api.updateStatus,
    enabled,
    retry: false,
  });
}
