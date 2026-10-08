import { useState } from "react";
import { ExternalLink, Loader2, X } from "lucide-react";
import { buttonVariants } from "@/components/ui/button";
import {
  Dialog,
  DialogBody,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from "@/components/ui/dialog";
import { kroner } from "@/components/NemligPicker";
import type { DeliveryDay, NemligSent } from "@/lib/api";
import { cn } from "@/lib/utils";

export interface NemligOrderProps {
  /** What sending the list did, or nothing while the sheet is closed. */
  sent?: NemligSent;
  days?: DeliveryDay[];
  daysError?: string;
  reserving?: number;
  reserveError?: string;
  /** When the reserved time is let go unless the order is placed. */
  heldUntil?: Date;
  onReserve: (slotId: number) => void;
  /** Open the picker for an item whose product would not go in. */
  onChooseAnother?: (name: string) => void;
  onClose: () => void;
}

/** "fre. 10. okt." in the reader's own language. */
function dayLabel(date: string): string {
  return new Date(`${date}T12:00:00`).toLocaleDateString(undefined, {
    weekday: "short",
    day: "numeric",
    month: "short",
  });
}

/** "by 14:00", or "by thu. 14:00" when that is not the delivery day. */
function orderBy(deadline: string, deliveryDate: string): string {
  const at = new Date(deadline);
  if (Number.isNaN(at.getTime())) return "";
  const time = at.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
  if (deadline.slice(0, 10) === deliveryDate) return `order by ${time}`;
  const day = at.toLocaleDateString(undefined, { weekday: "short" });
  return `order by ${day} ${time}`;
}

function hours(start: number, end: number): string {
  const pad = (h: number) => h.toString().padStart(2, "0");
  return `${pad(start)}–${pad(end)}`;
}

/**
 * After the list has gone to nemlig.com: what is in the basket, when it
 * should come, and the way to pay.
 *
 * Paying is not here on purpose. Niles fills the basket and holds a
 * delivery time; the order is placed at nemlig.com, by a person, where
 * the card and the final total are.
 */
export function NemligOrder({
  sent,
  days,
  daysError,
  reserving,
  reserveError,
  heldUntil,
  onReserve,
  onChooseAnother,
  onClose,
}: NemligOrderProps) {
  const basket = sent?.basket;
  const firstOpen = days?.find((d) => d.slots.some((s) => s.available))?.date;
  const reservedDay = days?.find((d) => d.slots.some((s) => s.selected))?.date;
  const [picked, setPicked] = useState<string>();
  const day = picked ?? reservedDay ?? firstOpen;
  const slots = days?.find((d) => d.date === day)?.slots ?? [];

  return (
    <Dialog
      open={sent !== undefined}
      onOpenChange={(open: boolean) => {
        if (!open) onClose();
      }}
    >
      <DialogContent
        initialFocus={(openType) => openType === "keyboard"}
        className="flex max-h-[90dvh] flex-col p-0"
      >
        {sent && basket && (
          <>
            <div className="border-border flex items-start gap-3 border-b px-4 py-3">
              <div className="min-w-0 flex-1">
                <DialogTitle>In your nemlig.com basket</DialogTitle>
                <DialogDescription>
                  {sent.sent} {sent.sent === 1 ? "item" : "items"} sent ·{" "}
                  {basket.lines.length} in the basket · {kroner(basket.total)}
                </DialogDescription>
              </div>
              <DialogClose
                aria-label="Close"
                className="text-muted-foreground hover:text-foreground focus-visible:ring-3 focus-visible:ring-ring/50 flex size-8 shrink-0 items-center justify-center rounded-lg focus-visible:outline-none"
              >
                <X aria-hidden className="size-4" />
              </DialogClose>
            </div>

            <DialogBody className="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto px-4 py-3">
              {sent.unavailable.length > 0 && (
                <div className="flex flex-col gap-1.5">
                  <p className="text-destructive text-sm">
                    Sold out at nemlig.com, so not in the basket:
                  </p>
                  <ul className="flex flex-wrap gap-1.5">
                    {sent.unavailable.map((name) => (
                      <li key={name}>
                        <button
                          type="button"
                          onClick={() => onChooseAnother?.(name)}
                          className="bg-muted hover:bg-muted/70 rounded-full px-3 py-1 text-sm focus-visible:ring-3 focus-visible:ring-ring/50 focus-visible:outline-none"
                        >
                          {name} — choose another
                        </button>
                      </li>
                    ))}
                  </ul>
                </div>
              )}
              {sent.without.length > 0 && (
                <p className="text-muted-foreground text-sm">
                  Not sent, with no product chosen: {sent.without.join(", ")}.
                </p>
              )}
              {!basket.meets_minimum && basket.minimum_total && (
                <p className="text-sm">
                  nemlig.com's smallest order is {kroner(basket.minimum_total)}; the
                  basket is at {kroner(basket.total)}.
                </p>
              )}

              <section className="flex flex-col gap-2">
                <h3 className="text-sm font-medium">Delivery</h3>
                {basket.delivery && (
                  <p className="text-sm">
                    Reserved: <span className="font-medium">{basket.delivery}</span>
                  </p>
                )}
                {/* nemlig lets a reserved time go after twenty minutes;
                    the clock time is easier to act on than a duration. */}
                {basket.delivery && heldUntil && (
                  <p className="text-muted-foreground text-sm">
                    Held until{" "}
                    {heldUntil.toLocaleTimeString(undefined, {
                      hour: "2-digit",
                      minute: "2-digit",
                    })}{" "}
                    — finish at nemlig.com before then, or the time is let go.
                  </p>
                )}
                {daysError && <p className="text-destructive text-sm">{daysError}</p>}
                {!days && !daysError && (
                  <Loader2 aria-label="Finding delivery times" className="size-4 animate-spin" />
                )}
                {days && (
                  <>
                    <div className="-mx-1 flex gap-1.5 overflow-x-auto px-1 pb-1" role="tablist">
                      {days.map((d) => (
                        <button
                          key={d.date}
                          type="button"
                          role="tab"
                          aria-selected={d.date === day}
                          disabled={!d.slots.some((s) => s.available)}
                          onClick={() => setPicked(d.date)}
                          className={cn(
                            "shrink-0 rounded-full px-3 py-1.5 text-sm transition-colors disabled:opacity-40",
                            "focus-visible:ring-3 focus-visible:ring-ring/50 focus-visible:outline-none",
                            d.date === day ? "bg-primary text-primary-foreground" : "bg-muted",
                          )}
                        >
                          {dayLabel(d.date)}
                        </button>
                      ))}
                    </div>
                    <ul className="grid grid-cols-2 gap-1.5 sm:grid-cols-3">
                      {slots.map((slot) => (
                        <li key={slot.id}>
                          <button
                            type="button"
                            aria-pressed={slot.selected}
                            disabled={!slot.available || reserving !== undefined}
                            onClick={() => onReserve(slot.id)}
                            className={cn(
                              "flex w-full flex-col items-start rounded-lg border px-3 py-2 text-left text-sm transition-colors disabled:opacity-40",
                              "hover:bg-muted/60 focus-visible:ring-3 focus-visible:ring-ring/50 focus-visible:outline-none",
                              slot.selected && "border-primary bg-primary/5",
                            )}
                          >
                            <span className="flex w-full items-center justify-between font-medium">
                              {hours(slot.start_hour, slot.end_hour)}
                              {reserving === slot.id && (
                                <Loader2 aria-hidden className="size-3.5 animate-spin" />
                              )}
                            </span>
                            <span className="text-muted-foreground text-xs">
                              {kroner(slot.price)}
                            </span>
                            {slot.available && slot.deadline && day && (
                              <span className="text-muted-foreground text-[11px]">
                                {orderBy(slot.deadline, day)}
                              </span>
                            )}
                          </button>
                        </li>
                      ))}
                    </ul>
                  </>
                )}
                {reserveError && <p className="text-destructive text-sm">{reserveError}</p>}
              </section>
            </DialogBody>

            <div className="border-border border-t px-4 py-3">
              {/* A link, not a fetch: the order is placed on nemlig.com's
                  own page, signed in there, by whoever is holding the
                  phone. */}
              <a
                href={sent.checkout}
                target="_blank"
                rel="noopener noreferrer"
                className={cn(buttonVariants(), "w-full")}
              >
                Finish at nemlig.com
                <ExternalLink aria-hidden />
              </a>
            </div>
          </>
        )}
      </DialogContent>
    </Dialog>
  );
}
