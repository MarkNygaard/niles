import { useRef, useState } from "react";
import { GripVertical } from "lucide-react";
import { cn } from "@/lib/utils";

export interface OrderListProps {
  /** What is arranged, in the order it is arranged in. */
  items: { name: string; label: string }[];
  disabled?: boolean;
  /** The arrangement, by name, once it has been changed. */
  onChange: (order: string[]) => void;
  /** What sits at the end of a row: a number, a switch. */
  trailing?: (name: string, index: number) => React.ReactNode;
  /** A row drawn faded — one that is there but switched off. */
  muted?: (name: string) => boolean;
}

/** Move `from` to `to`, leaving everything else in its relative place. */
export function moved<T>(list: T[], from: number, to: number): T[] {
  const next = [...list];
  const [item] = next.splice(from, 1);
  next.splice(Math.min(Math.max(to, 0), list.length - 1), 0, item);
  return next;
}

/**
 * A list arranged by dragging, for the rooms and the menu alike.
 *
 * Dragged by a grip, and moved by the arrow keys when that grip has
 * focus: the same control serving both, rather than a pair of nudge
 * buttons beside every row. Each move is saved as it happens — there is
 * nothing here to get half-right and no reason to make somebody confirm
 * a list they can see.
 */
export function OrderList({ items, disabled, onChange, trailing, muted }: OrderListProps) {
  const [draft, setDraft] = useState<string[] | null>(null);
  // State rather than a ref: the row being carried is drawn differently
  // while it is, and a ref changing draws nothing.
  const [held, setHeld] = useState<number | null>(null);
  const list = useRef<HTMLUListElement | null>(null);

  // The arrangement being dragged, or the one on screen when nothing
  // is. Held only for the length of a drag: afterwards the saved order
  // is the truth, and keeping a copy is how the two drift.
  const order = draft ?? items.map((item) => item.name);
  const labels = new Map(items.map((item) => [item.name, item.label]));

  /** Which row a pointer at `clientY` is over. */
  function rowAt(clientY: number): number {
    const box = list.current?.getBoundingClientRect();
    if (!box || order.length === 0) return 0;
    const row = box.height / order.length;
    const index = Math.floor((clientY - box.top) / row);
    return Math.min(Math.max(index, 0), order.length - 1);
  }

  function move(at: number, to: number) {
    const next = moved(order, at, to);
    setDraft(next);
    return next;
  }

  return (
    <ul ref={list} className="flex flex-col gap-1">
      {order.map((name, index) => (
        <li
          key={name}
          className={cn(
            "bg-card flex items-center gap-3 rounded-lg border px-3 py-2.5",
            held === index && "border-ring shadow-sm",
          )}
        >
          <button
            type="button"
            disabled={disabled}
            aria-label={`Move ${labels.get(name) ?? name}`}
            className={cn(
              "text-muted-foreground hover:text-foreground -m-1 cursor-grab touch-none p-1",
              "focus-visible:ring-3 focus-visible:ring-ring/50 rounded focus-visible:outline-none",
              "disabled:cursor-not-allowed disabled:opacity-50",
            )}
            onPointerDown={(e) => {
              e.currentTarget.setPointerCapture(e.pointerId);
              setHeld(index);
            }}
            onPointerMove={(e) => {
              if (held === null) return;
              const to = rowAt(e.clientY);
              if (to === held) return;
              move(held, to);
              setHeld(to);
            }}
            onPointerUp={() => {
              setHeld(null);
              // Nothing to save when the row was picked up and put back
              // down where it started.
              if (draft) onChange(draft);
              setDraft(null);
            }}
            onKeyDown={(e) => {
              const by =
                e.key === "ArrowUp" ? -1 : e.key === "ArrowDown" ? 1 : 0;
              if (by === 0) return;
              e.preventDefault();
              const next = move(index, index + by);
              setDraft(null);
              onChange(next);
            }}
          >
            <GripVertical aria-hidden className="size-4" />
          </button>

          <span
            className={cn(
              "min-w-0 flex-1 truncate text-sm font-medium",
              muted?.(name) && "text-muted-foreground",
            )}
          >
            {labels.get(name) ?? name}
          </span>
          {trailing?.(name, index)}
        </li>
      ))}
    </ul>
  );
}
