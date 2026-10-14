import { Gauge } from "lucide-react";
import { humanize, readingsLine } from "@/lib/rooms";
import type { Device } from "@/lib/api";

/**
 * Something in the room that only measures: its name and what it
 * reads, `340 W · 12.4 kWh`. Nothing to press — a dishwasher's plug
 * that is not a lamp is kept out of the light controls on purpose.
 */
export function MeterRow({ device }: { device: Device }) {
  return (
    <div className="flex items-center gap-3 py-3">
      <span
        aria-hidden
        className="bg-muted text-muted-foreground flex size-10 shrink-0 items-center justify-center rounded-full [&_svg]:size-4"
      >
        <Gauge />
      </span>
      <div className="min-w-0 flex-1">
        <div className="truncate text-sm font-medium">{humanize(device.name)}</div>
        <div className="text-muted-foreground text-xs">{readingsLine(device)}</div>
      </div>
    </div>
  );
}
