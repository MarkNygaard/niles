import { ShoppingBasket } from "lucide-react";
import { kroner } from "@/components/NemligPicker";
import type { NemligOrder } from "@/lib/api";

export interface GroceryDeliveryCardProps {
  /** The next nemlig.com order still to arrive, if any. */
  order?: NemligOrder | null;
  /** Today, the way the order's dates are written: "2026-10-10". */
  today: string;
}

/** "07:00" out of "2026-10-10T07:00:00". */
function clock(at: string): string {
  return at.slice(11, 16);
}

/** Whether the order comes on `today`. Its times are Danish local time,
    written without a zone, so the date is the first ten characters. */
export function arrivesToday(order: NemligOrder | null | undefined, today: string): boolean {
  return Boolean(order?.delivery_start?.startsWith(today));
}

/**
 * On the day a nemlig.com order arrives, when — on the page somebody
 * opens in the morning anyway. Nothing on any other day.
 */
export function GroceryDeliveryCard({ order, today }: GroceryDeliveryCardProps) {
  if (!order || !arrivesToday(order, today)) return null;
  const from = clock(order.delivery_start!);
  const to = order.delivery_end ? clock(order.delivery_end) : undefined;
  return (
    <div className="bg-card flex items-center gap-3 rounded-xl px-4 py-3">
      <ShoppingBasket aria-hidden className="text-primary size-5 shrink-0" />
      <div className="min-w-0 flex-1">
        <div className="text-sm font-medium">
          Groceries arrive today, {from}
          {to && `–${to}`}
        </div>
        {order.total !== null && (
          <div className="text-muted-foreground text-xs">
            nemlig.com · {kroner(order.total)}
          </div>
        )}
      </div>
    </div>
  );
}
