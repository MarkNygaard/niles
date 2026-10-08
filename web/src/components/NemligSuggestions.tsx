import { useEffect, useState } from "react";
import { Thumbnail, kroner } from "@/components/NemligPicker";
import type { NemligProduct } from "@/lib/api";
import { cn } from "@/lib/utils";

export interface NemligSuggestionsProps {
  /** What is being typed into the add field. */
  query: string;
  search: (query: string) => Promise<NemligProduct[]>;
  onChoose: (product: NemligProduct) => void;
}

/** Long enough that a word is being typed, not each letter of it. */
const PAUSE_MS = 350;
const SHOWN = 4;

/**
 * nemlig.com's products for what is being typed, to add with the product
 * already chosen.
 *
 * Searched once typing pauses, not per keystroke: each search is a round
 * trip to nemlig, and "letmælk" typed letter by letter would be seven.
 */
export function NemligSuggestions({ query, search, onChoose }: NemligSuggestionsProps) {
  const [found, setFound] = useState<{ query: string; products: NemligProduct[] }>();
  const wanted = query.trim();

  useEffect(() => {
    if (wanted.length < 2) return;
    let current = true;
    const timer = window.setTimeout(() => {
      search(wanted)
        .then((products) => current && setFound({ query: wanted, products }))
        // A suggestion that fails is a suggestion not made; the field
        // still adds what was typed.
        .catch(() => current && setFound({ query: wanted, products: [] }));
    }, PAUSE_MS);
    return () => {
      current = false;
      window.clearTimeout(timer);
    };
  }, [wanted, search]);

  // Only what was found for the words in the field now.
  if (wanted.length < 2 || found?.query !== wanted || found.products.length === 0) return null;

  return (
    <ul className="bg-card -mt-3 overflow-hidden rounded-xl border" aria-label="From nemlig.com">
      {found.products.slice(0, SHOWN).map((product) => (
        <li key={product.id} className="border-border/60 border-b last:border-b-0">
          <button
            type="button"
            onClick={() => onChoose(product)}
            className={cn(
              "flex w-full items-center gap-3 px-3 py-2 text-left transition-colors",
              "hover:bg-muted/50 focus-visible:ring-3 focus-visible:ring-ring/50 focus-visible:-outline-offset-2 focus-visible:outline-none",
              !product.available && "opacity-60",
            )}
          >
            <Thumbnail product={product} className="size-10" />
            <span className="min-w-0 flex-1">
              <span className="block truncate text-sm font-medium">{product.name}</span>
              <span className="text-muted-foreground block truncate text-xs">
                {product.description}
              </span>
            </span>
            <span className="shrink-0 text-right text-sm">
              <span className="block font-medium">{kroner(product.price)}</span>
              {product.offer && (
                <span className="text-primary block text-xs">On offer</span>
              )}
            </span>
          </button>
        </li>
      ))}
    </ul>
  );
}
