import { Lock } from "lucide-react";
import { OrderList } from "@/components/OrderList";
import { Switch } from "@/components/ui/switch";
import type { MenuEntry, MenuItem } from "@/lib/menu";

export interface MenuCardProps {
  /** The entries between Home and Me, from `menuOf`. */
  menu: MenuItem[];
  disabled?: boolean;
  /** The whole arrangement, once anything in it has changed. */
  onChange: (next: { order: MenuEntry[]; hidden: MenuEntry[] }) => void;
}

const LABELS: Record<MenuEntry, string> = {
  groceries: "Groceries",
  chat: "Chat",
};

/** Home and Me, drawn as rows so the list reads as the whole menu,
    with a lock where the others have a grip. */
function Fixed({ label }: { label: string }) {
  return (
    <div className="bg-card flex items-center gap-3 rounded-lg border px-3 py-2.5">
      <Lock aria-hidden className="text-muted-foreground size-4" />
      <span className="min-w-0 flex-1 truncate text-sm font-medium">{label}</span>
      <span className="text-muted-foreground text-xs">Always shown</span>
    </div>
  );
}

/**
 * Which pages the menu leads to, and in what order.
 *
 * Home stays first and Me last: Home is where the app opens, and Me is
 * the way back here to undo a hidden entry. Hiding takes an entry out of
 * the menu only — its page, and what voice does with it, carry on.
 */
export function MenuCard({ menu, disabled, onChange }: MenuCardProps) {
  const hidden = new Set(menu.filter((item) => item.hidden).map((item) => item.id));
  const order = menu.map((item) => item.id);

  return (
    <div className="flex flex-col gap-1">
      <Fixed label="Home" />
      <OrderList
        items={order.map((id) => ({ name: id, label: LABELS[id] }))}
        disabled={disabled}
        muted={(name) => hidden.has(name as MenuEntry)}
        onChange={(next) => onChange({ order: next as MenuEntry[], hidden: [...hidden] })}
        trailing={(name) => {
          const id = name as MenuEntry;
          return (
            <Switch
              aria-label={`Show ${LABELS[id]}`}
              checked={!hidden.has(id)}
              disabled={disabled}
              onCheckedChange={(shown) =>
                onChange({
                  order,
                  hidden: order.filter((other) => (other === id ? !shown : hidden.has(other))),
                })
              }
            />
          );
        }}
      />
      <Fixed label="Me" />
    </div>
  );
}
