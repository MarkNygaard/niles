import { OrderList } from "@/components/OrderList";
import { Switch } from "@/components/ui/switch";

export interface RoomOrderCardProps {
  /** Every room the dashboard can show, in the order it shows them. */
  rooms: { name: string; label: string }[];
  /** The rooms left off the dashboard, by name. */
  hidden?: string[];
  disabled?: boolean;
  /** The arrangement, by room name, once it has been changed. */
  onChange: (order: string[]) => void;
  /** Every hidden room, by name, once one has been shown or hidden. */
  onHiddenChange: (hidden: string[]) => void;
}

/**
 * The order the rooms sit in on the dashboard, and which of them it
 * shows.
 *
 * A house has an order its rooms are thought about in — the one you
 * walk through, or the one you use most — and it is never the alphabet,
 * which is what the dashboard sorted by. So this is a list you arrange
 * rather than a setting you type.
 *
 * A hidden room stays in the list, faded, with its switch: this is the
 * one place it can be shown again from, and it keeps its place in the
 * order while it is away.
 */
export function RoomOrderCard({
  rooms,
  hidden = [],
  disabled,
  onChange,
  onHiddenChange,
}: RoomOrderCardProps) {
  if (rooms.length === 0) {
    return (
      <p className="text-muted-foreground text-sm">
        No rooms yet. A room appears here once something in it is paired.
      </p>
    );
  }
  const away = new Set(hidden);
  const labels = new Map(rooms.map((room) => [room.name, room.label]));
  return (
    <OrderList
      items={rooms}
      disabled={disabled}
      onChange={onChange}
      muted={(name) => away.has(name)}
      trailing={(name, index) => (
        <>
          <span className="text-muted-foreground text-xs tabular-nums">{index + 1}</span>
          <Switch
            aria-label={`Show ${labels.get(name) ?? name} on the dashboard`}
            checked={!away.has(name)}
            disabled={disabled}
            onCheckedChange={(shown) => {
              const next = new Set(away);
              if (shown) next.delete(name);
              else next.add(name);
              onHiddenChange([...next]);
            }}
          />
        </>
      )}
    />
  );
}
