import { RotateCw } from "lucide-react";
import { useRestartStatus } from "@/hooks/useRestartStatus";
import { RestartButton } from "@/components/RestartButton";

/** The bar at the top of Settings while a change waits for a restart. */
export function RestartBar() {
  const { data } = useRestartStatus();
  if (!data || data.pending.length === 0) return null;
  const labels = data.pending.map((p) => p.label).join(", ");
  return (
    <div
      role="status"
      className="bg-muted/60 flex flex-wrap items-center gap-3 rounded-md px-3 py-2 text-sm"
    >
      <RotateCw className="size-4 shrink-0" aria-hidden />
      <span className="min-w-0 flex-1">
        Some changes take effect after a restart: {labels}.
        {!data.persistent &&
          " Without a database a restart would undo them too, so set them in niles.toml instead."}
        {data.persistent && !data.supervisor && " Restart Niles where it runs to apply them."}
        {data.restarting && " Restarting…"}
      </span>
      <RestartButton status={data} />
    </div>
  );
}
