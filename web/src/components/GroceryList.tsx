import { useState } from "react";
import { Check, Loader2, Plus, ShoppingCart, Trash2, X } from "lucide-react";
import { Thumbnail } from "@/components/NemligPicker";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import type { GroceryEdit, GroceryItem } from "@/lib/api";
import { cn } from "@/lib/utils";

export interface GroceryListProps {
  items: GroceryItem[];
  usual: string[];
  error?: string;
  onAdd: (name: string) => void;
  onToggle: (item: GroceryItem) => void;
  onEdit: (item: GroceryItem, edit: GroceryEdit) => void;
  onRemove: (item: GroceryItem) => void;
  onClear: () => void;
  /** Choose the nemlig.com product for an item. Absent when nemlig.com
      is not switched on, and then there is no button for it. */
  onPick?: (item: GroceryItem) => void;
  /** Put the list in the nemlig.com basket. Absent like `onPick`. */
  onSend?: () => void;
  sending?: boolean;
}

/**
 * The list as it is used in a shop: what is left at the top, a tap to
 * put it in the basket, and what is in the basket out of the way below.
 */
export function GroceryList({
  items,
  usual,
  error,
  onAdd,
  onToggle,
  onEdit,
  onRemove,
  onClear,
  onPick,
  onSend,
  sending,
}: GroceryListProps) {
  const [draft, setDraft] = useState("");
  const [editing, setEditing] = useState<number | null>(null);
  const toBuy = items.filter((i) => !i.checked_at);
  const basket = items.filter((i) => i.checked_at);
  const linked = toBuy.filter((i) => i.nemlig).length;

  const submit = (event: React.FormEvent) => {
    event.preventDefault();
    const name = draft.trim();
    if (!name) return;
    onAdd(name);
    setDraft("");
  };

  return (
    <div className="flex flex-col gap-5">
      <form onSubmit={submit} className="flex gap-2">
        <Input
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          placeholder="Add to the list"
          aria-label="Add to the list"
          // An English keyboard "corrects" skyr to sky.
          autoCorrect="off"
          className="h-10"
        />
        <Button type="submit" size="icon-lg" aria-label="Add" disabled={!draft.trim()}>
          <Plus />
        </Button>
      </form>

      {error && <p className="text-destructive text-sm">{error}</p>}

      {usual.length > 0 && (
        <div>
          <Heading>Usual</Heading>
          <div className="flex flex-wrap gap-1.5">
            {usual.map((name) => (
              <button
                key={name}
                type="button"
                onClick={() => onAdd(name)}
                aria-label={`Add ${name}`}
                className={cn(
                  "bg-card text-foreground flex items-center gap-1 rounded-full px-3 py-1.5 text-sm transition-colors",
                  "hover:bg-muted focus-visible:ring-3 focus-visible:ring-ring/50 focus-visible:outline-none",
                )}
              >
                <Plus aria-hidden className="text-muted-foreground size-3.5" />
                {name}
              </button>
            ))}
          </div>
        </div>
      )}

      {toBuy.length === 0 ? (
        <p className="text-muted-foreground px-1 text-sm">
          Nothing to buy. Type it above, or tell Niles to “add milk to the list”.
        </p>
      ) : (
        <ul className="bg-card overflow-hidden rounded-xl">
          {toBuy.map((item) =>
            editing === item.id ? (
              <Editor
                key={item.id}
                item={item}
                onSave={(edit) => {
                  onEdit(item, edit);
                  setEditing(null);
                }}
                onRemove={() => {
                  onRemove(item);
                  setEditing(null);
                }}
                onCancel={() => setEditing(null)}
              />
            ) : (
              <Row
                key={item.id}
                item={item}
                onToggle={() => onToggle(item)}
                onOpen={() => setEditing(item.id)}
                onPick={onPick && (() => onPick(item))}
              />
            ),
          )}
        </ul>
      )}

      {onSend && linked > 0 && (
        <Button onClick={onSend} disabled={sending} className="self-start">
          {sending ? <Loader2 className="animate-spin" /> : <ShoppingCart />}
          Send {linked} to nemlig.com
        </Button>
      )}

      {basket.length > 0 && (
        <div>
          <div className="flex items-center justify-between">
            <Heading>In the basket</Heading>
            <Button variant="ghost" size="sm" onClick={onClear} className="-mt-1">
              Clear
            </Button>
          </div>
          <ul className="bg-card overflow-hidden rounded-xl">
            {basket.map((item) => (
              <Row key={item.id} item={item} onToggle={() => onToggle(item)} />
            ))}
          </ul>
        </div>
      )}
    </div>
  );
}

function Heading({ children }: { children: React.ReactNode }) {
  return (
    <div className="text-muted-foreground mb-1 px-1 text-xs font-medium tracking-wide uppercase">
      {children}
    </div>
  );
}

function Row({
  item,
  onToggle,
  onOpen,
  onPick,
}: {
  item: GroceryItem;
  onToggle: () => void;
  /** Absent in the basket: what has been bought is not renamed. */
  onOpen?: () => void;
  onPick?: () => void;
}) {
  const checked = Boolean(item.checked_at);
  const label = (
    <span className="min-w-0 flex-1">
      <span
        className={cn(
          "block truncate text-sm",
          checked && "text-muted-foreground line-through",
        )}
      >
        {item.name}
        {item.quantity && <span className="text-muted-foreground"> · {item.quantity}</span>}
      </span>
      {/* Shown until it is bought, so a wrong guess can be fixed
          before it is learned. */}
      {item.said && !checked && (
        <span className="text-muted-foreground block truncate text-xs">
          asked for as “{item.said}”
        </span>
      )}
    </span>
  );

  return (
    <li className="border-border/60 flex items-center gap-1 border-b px-3 py-1.5 last:border-b-0">
      <button
        type="button"
        role="checkbox"
        aria-checked={checked}
        aria-label={item.name}
        onClick={onToggle}
        className={cn(
          // A shop is not a desk: the target is the whole height of the
          // row, not the circle drawn in it.
          "-my-1.5 -ml-3 flex h-12 w-12 shrink-0 items-center justify-center",
          "focus-visible:ring-3 focus-visible:ring-ring/50 focus-visible:-outline-offset-2 focus-visible:outline-none",
        )}
      >
        <span
          className={cn(
            "flex size-5 items-center justify-center rounded-full border transition-colors",
            checked ? "border-primary bg-primary text-primary-foreground" : "border-muted-foreground/50",
          )}
        >
          {checked && <Check className="size-3.5" />}
        </span>
      </button>
      {onOpen ? (
        <button
          type="button"
          onClick={onOpen}
          aria-label={`Edit ${item.name}`}
          className="min-w-0 flex-1 py-1.5 text-left focus-visible:outline-none"
        >
          {label}
        </button>
      ) : (
        label
      )}
      {onPick && (
        <button
          type="button"
          onClick={onPick}
          aria-label={
            item.nemlig
              ? `${item.nemlig.name} at nemlig.com — change`
              : `Choose ${item.name} at nemlig.com`
          }
          className={cn(
            "-my-1 flex size-10 shrink-0 items-center justify-center rounded-lg transition-colors",
            "hover:bg-muted focus-visible:ring-3 focus-visible:ring-ring/50 focus-visible:outline-none",
          )}
        >
          {item.nemlig ? (
            <Thumbnail product={item.nemlig} className="size-9" />
          ) : (
            <ShoppingCart aria-hidden className="text-muted-foreground size-4" />
          )}
        </button>
      )}
    </li>
  );
}

function Editor({
  item,
  onSave,
  onRemove,
  onCancel,
}: {
  item: GroceryItem;
  onSave: (edit: GroceryEdit) => void;
  onRemove: () => void;
  onCancel: () => void;
}) {
  const [name, setName] = useState(item.name);
  const [quantity, setQuantity] = useState(item.quantity ?? "");

  const submit = (event: React.FormEvent) => {
    event.preventDefault();
    if (!name.trim()) return;
    onSave({ name: name.trim(), quantity: quantity.trim() });
  };

  return (
    <li className="border-border/60 border-b p-2 last:border-b-0">
      <form
        onSubmit={submit}
        onKeyDown={(e) => e.key === "Escape" && onCancel()}
        className="flex flex-col gap-2"
      >
        <div className="flex gap-2">
          <Input
            value={name}
            onChange={(e) => setName(e.target.value)}
            aria-label="Name"
            autoFocus
            autoCorrect="off"
            className="h-9 flex-1"
          />
          <Input
            value={quantity}
            onChange={(e) => setQuantity(e.target.value)}
            aria-label="Quantity"
            placeholder="How many"
            className="h-9 w-24"
          />
        </div>
        <div className="flex items-center gap-2">
          <Button type="submit" size="sm" disabled={!name.trim()}>
            Save
          </Button>
          <Button type="button" variant="ghost" size="sm" onClick={onCancel}>
            <X />
            Cancel
          </Button>
          <Button
            type="button"
            variant="destructive"
            size="sm"
            onClick={onRemove}
            className="ml-auto"
          >
            <Trash2 />
            Remove
          </Button>
        </div>
      </form>
    </li>
  );
}
