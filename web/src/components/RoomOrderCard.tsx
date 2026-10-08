import { OrderList } from "@/components/OrderList";

export interface RoomOrderCardProps {
  /** Every room the dashboard shows, in the order it shows them. */
  rooms: { name: string; label: string }[];
  disabled?: boolean;
  /** The arrangement, by room name, once it has been changed. */
  onChange: (order: string[]) => void;
}

/**
 * The order the rooms sit in on the dashboard.
 *
 * A house has an order its rooms are thought about in — the one you
 * walk through, or the one you use most — and it is never the alphabet,
 * which is what the dashboard sorted by. So this is a list you arrange
 * rather than a setting you type.
 */
export function RoomOrderCard({ rooms, disabled, onChange }: RoomOrderCardProps) {
  if (rooms.length === 0) {
    return (
      <p className="text-muted-foreground text-sm">
        No rooms yet. A room appears here once something in it is paired.
      </p>
    );
  }
  return (
    <OrderList
      items={rooms}
      disabled={disabled}
      onChange={onChange}
      trailing={(_, index) => (
        <span className="text-muted-foreground text-xs tabular-nums">{index + 1}</span>
      )}
    />
  );
}
