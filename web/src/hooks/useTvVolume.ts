import { useMutation, useQueryClient } from "@tanstack/react-query";
import { afterChange, tvVolume, withTvVolume } from "@/components/TvRow";
import { api } from "@/lib/api";
import type { TvInfo, TvVolumeChange } from "@/lib/api";

/**
 * A change to the TV's own speakers. Drawn at once, and corrected by
 * what the TV answers: a TV takes a moment, and a slider that waits for
 * it feels broken.
 */
export function useTvVolume(
  handlers: { onSuccess?: () => void; onError?: (failure: Error) => void } = {},
) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (change: TvVolumeChange) => api.tvVolume(change),
    onMutate: async (change) => {
      await queryClient.cancelQueries({ queryKey: ["tv"] });
      queryClient.setQueryData<TvInfo>(["tv"], (tv) => {
        const now = tvVolume(tv);
        return now ? withTvVolume(tv, afterChange(now, change)) : tv;
      });
    },
    onSuccess: (after) => {
      handlers.onSuccess?.();
      queryClient.setQueryData<TvInfo>(["tv"], (tv) => withTvVolume(tv, after));
    },
    onError: (failure) => {
      handlers.onError?.(failure);
      queryClient.invalidateQueries({ queryKey: ["tv"] });
    },
  });
}
