import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Check, Loader2, Search, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogBody,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Skeleton } from "@/components/ui/skeleton";
import type { GroceryItem, NemligProduct } from "@/lib/api";
import { cn } from "@/lib/utils";

export interface NemligPickerProps {
  /** The item being matched, or nothing when the picker is closed. */
  item?: GroceryItem;
  search: (query: string) => Promise<NemligProduct[]>;
  onChoose: (item: GroceryItem, product: NemligProduct | null) => void;
  onClose: () => void;
}

/** "14,95 kr" — the way the shop writes it. */
export function kroner(price: number): string {
  return `${price.toFixed(2).replace(".", ",")} kr`;
}

/**
 * Which of nemlig.com's products an item on the list is.
 *
 * Searched by the item's name to begin with, and searchable again,
 * because "rundstykker" finds frozen ones first and somebody may want
 * the fresh. The choice is remembered for the next time the name is
 * added, so this is opened once per product, not once per shop.
 */
export function NemligPicker({ item, search, onChoose, onClose }: NemligPickerProps) {
  return (
    <Dialog
      open={item !== undefined}
      onOpenChange={(open: boolean) => {
        if (!open) onClose();
      }}
    >
      <DialogContent
        // Focus only when a keyboard opened it; a tap otherwise leaves a
        // ring round the close button. As the room sheets do.
        initialFocus={(openType) => openType === "keyboard"}
        // A fixed height, not a maximum: sized to its contents it opened
        // short and jumped taller when the results came in.
        className="flex h-[85dvh] flex-col p-0 sm:h-[min(80vh,44rem)]"
      >
        {item && (
          <Picker
            key={item.id}
            item={item}
            search={search}
            onChoose={(product) => onChoose(item, product)}
          />
        )}
      </DialogContent>
    </Dialog>
  );
}

function Picker({
  item,
  search,
  onChoose,
}: {
  item: GroceryItem;
  search: (query: string) => Promise<NemligProduct[]>;
  onChoose: (product: NemligProduct | null) => void;
}) {
  const [draft, setDraft] = useState(item.name);
  const [query, setQuery] = useState(item.name);
  const results = useQuery({
    queryKey: ["nemlig-search", query],
    queryFn: () => search(query),
    enabled: query.trim().length > 0,
    // The shelves do not change while somebody is choosing.
    staleTime: 5 * 60_000,
    retry: false,
  });

  return (
    <>
      <div className="border-border flex items-start gap-3 border-b px-4 py-3">
        <div className="min-w-0 flex-1">
          <DialogTitle>{item.name} at nemlig.com</DialogTitle>
          <DialogDescription>
            The one you pick goes in the basket, and is picked again next time.
          </DialogDescription>
        </div>
        <DialogClose
          aria-label="Close"
          className="text-muted-foreground hover:text-foreground focus-visible:ring-3 focus-visible:ring-ring/50 flex size-8 shrink-0 items-center justify-center rounded-lg focus-visible:outline-none"
        >
          <X aria-hidden className="size-4" />
        </DialogClose>
      </div>

      <form
        className="flex gap-2 px-4 pt-3"
        onSubmit={(e) => {
          e.preventDefault();
          setQuery(draft.trim());
        }}
      >
        <Input
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          aria-label="Search nemlig.com"
          autoCorrect="off"
          className="h-9"
        />
        <Button type="submit" variant="outline" size="icon-lg" aria-label="Search">
          {results.isFetching ? <Loader2 className="animate-spin" /> : <Search />}
        </Button>
      </form>

      <DialogBody className="min-h-0 flex-1 overflow-y-auto px-4 py-3">
        {results.isError && (
          <p className="text-destructive px-2 py-3 text-sm">{results.error.message}</p>
        )}
        {results.data?.length === 0 && (
          <p className="text-muted-foreground px-2 py-3 text-sm">
            nemlig.com has nothing called “{query}”. Try another word.
          </p>
        )}
        {/* Two to a row with the picture large, the way a shop shows
            them: telling frozen rolls from fresh is a matter of looking. */}
        <ul className="grid grid-cols-2 gap-2 sm:grid-cols-3" aria-busy={results.isPending}>
          {results.isPending
            ? Array.from({ length: 6 }, (_, i) => <CardSkeleton key={i} />)
            : results.data?.map((product) => (
                <li key={product.id}>
                  <Card
                    product={product}
                    chosen={item.nemlig?.id === product.id}
                    onChoose={() => onChoose(product)}
                  />
                </li>
              ))}
        </ul>
      </DialogBody>

      {item.nemlig && (
        <div className="border-border border-t px-4 py-2">
          <Button variant="ghost" size="sm" onClick={() => onChoose(null)}>
            Don't use nemlig.com for this
          </Button>
        </div>
      )}
    </>
  );
}

/** One product, as a square picture with what it is underneath. */
function Card({
  product,
  chosen,
  onChoose,
}: {
  product: NemligProduct;
  chosen: boolean;
  onChoose: () => void;
}) {
  return (
    <button
      type="button"
      aria-pressed={chosen}
      onClick={onChoose}
      className={cn(
        "bg-card flex h-full w-full flex-col gap-2 rounded-xl border p-2 text-left transition-colors",
        "hover:bg-muted/40 focus-visible:ring-3 focus-visible:ring-ring/50 focus-visible:outline-none",
        chosen ? "border-primary ring-primary ring-1" : "border-border",
        // Still choosable: sold out today can be what is usually bought.
        !product.available && "opacity-60",
      )}
    >
      <span className="relative block">
        <Thumbnail product={product} className="aspect-square w-full p-1" />
        {chosen && (
          <span className="bg-primary text-primary-foreground absolute top-1.5 right-1.5 flex size-6 items-center justify-center rounded-full">
            <Check aria-hidden className="size-3.5" />
          </span>
        )}
      </span>
      <span className="flex min-w-0 flex-1 flex-col gap-0.5 px-0.5">
        <span className="line-clamp-2 text-sm leading-snug font-medium">{product.name}</span>
        <span className="text-muted-foreground truncate text-xs">{product.description}</span>
        <span className="mt-auto pt-1 text-sm">
          <span className="font-semibold">{kroner(product.price)}</span>
          {product.unit_price && (
            <span className="text-muted-foreground text-xs"> · {product.unit_price}</span>
          )}
        </span>
        {!product.available && <span className="text-destructive text-xs">Not available</span>}
      </span>
    </button>
  );
}

/** A card's shape before there is a product to put in it. */
function CardSkeleton() {
  return (
    <li aria-hidden className="flex flex-col gap-2 rounded-xl border p-2">
      <Skeleton className="aspect-square w-full rounded-lg" />
      <Skeleton className="h-4 w-4/5" />
      <Skeleton className="h-3 w-3/5" />
      <Skeleton className="h-4 w-2/5" />
    </li>
  );
}

/** The product's picture, or a plain tile where the shop has none. */
export function Thumbnail({
  product,
  className,
}: {
  product: NemligProduct;
  className?: string;
}) {
  return product.image ? (
    <img
      src={product.image}
      alt=""
      loading="lazy"
      // Nothing about this house travels to nemlig.com with a request
      // for a picture of milk.
      referrerPolicy="no-referrer"
      className={cn("shrink-0 rounded-md bg-white object-contain", className)}
    />
  ) : (
    <span aria-hidden className={cn("bg-muted shrink-0 rounded-md", className)} />
  );
}
