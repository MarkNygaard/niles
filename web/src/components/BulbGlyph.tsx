import { cn } from "@/lib/utils";

/**
 * A light, drawn as a light.
 *
 * Solid, not outlined, and that is the whole point of it. These are
 * read-outs — the card around them is the switch — so the colour *is*
 * the state, and a stroked glyph has too little area to carry a colour
 * at 20px. What used to be here was a power symbol on a filled disc,
 * which put the colour in the disc because the glyph could not hold it;
 * one of those reads as a switch, and twenty of them read as a warning
 * panel rather than a house with its lights on.
 *
 * Bootstrap Icons' `lightbulb-fill` (MIT), on a 16 viewBox as they draw
 * it. Lucide's bulb is stroked and disappears at this size.
 */
export function BulbGlyph({ className }: { className?: string }) {
  return (
    <svg
      viewBox="0 0 16 16"
      fill="currentColor"
      className={cn("shrink-0", className)}
      aria-hidden
      focusable="false"
    >
      <path d="M2 6a6 6 0 1 1 10.174 4.31c-.203.196-.359.4-.453.619l-.762 1.769A.5.5 0 0 1 10.5 13h-5a.5.5 0 0 1-.46-.302l-.761-1.77a2 2 0 0 0-.453-.618A5.98 5.98 0 0 1 2 6m3 8.5a.5.5 0 0 1 .5-.5h5a.5.5 0 0 1 0 1l-.224.447a1 1 0 0 1-.894.553H6.618a1 1 0 0 1-.894-.553L5.5 15a.5.5 0 0 1-.5-.5" />
    </svg>
  );
}
