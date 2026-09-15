import { cn } from "@/lib/utils";

/**
 * A light, drawn as a light.
 *
 * Solid, and that is the whole point of it. This one is a read-out —
 * the bar around it is the switch — so the colour *is* the state, and
 * a stroked glyph has too little area to carry a colour at 32px. What
 * used to be here was a power symbol on a filled disc, which put the
 * colour in the disc because the glyph could not hold it; one of those
 * reads as a switch, and twenty of them read as a warning panel rather
 * than a house with its lights on.
 *
 * Ant Design's `bulb` filled (MIT), on their 1024 viewBox.
 */
export function BulbGlyph({ className }: { className?: string }) {
  return (
    <svg
      viewBox="0 0 1024 1024"
      fill="currentColor"
      className={cn("shrink-0", className)}
      aria-hidden
      focusable="false"
    >
      <path d="M348 676.1C250 619.4 184 513.4 184 392c0-181.1 146.9-328 328-328s328 146.9 328 328c0 121.4-66 227.4-164 284.1V792c0 17.7-14.3 32-32 32H380c-17.7 0-32-14.3-32-32V676.1zM392 888h240c4.4 0 8 3.6 8 8v32c0 17.7-14.3 32-32 32H416c-17.7 0-32-14.3-32-32v-32c0-4.4 3.6-8 8-8z" />
    </svg>
  );
}

/**
 * The same light, hollow.
 *
 * For the room card, where the glyph is pressed into the card's own
 * colour rather than carrying one of its own: at half-strength white a
 * solid bulb is a blob, and the outline is what keeps the shape
 * readable at the size a tile's footer allows.
 *
 * Ant Design's `bulb` outlined (MIT), the same drawing as its filled
 * twin — which is why the two read as one light in two states rather
 * than as two icons.
 */
export function BulbOutlineGlyph({ className }: { className?: string }) {
  return (
    <svg
      viewBox="0 0 1024 1024"
      fill="currentColor"
      className={cn("shrink-0", className)}
      aria-hidden
      focusable="false"
    >
      <path d="M632 888H392c-4.4 0-8 3.6-8 8v32c0 17.7 14.3 32 32 32h192c17.7 0 32-14.3 32-32v-32c0-4.4-3.6-8-8-8zM512 64c-181.1 0-328 146.9-328 328 0 121.4 66 227.4 164 284.1V792c0 17.7 14.3 32 32 32h264c17.7 0 32-14.3 32-32V676.1c98-56.7 164-162.7 164-284.1 0-181.1-146.9-328-328-328zm127.9 549.8L604 634.6V752H420V634.6l-35.9-20.8C305.4 568.3 256 484.5 256 392c0-141.4 114.6-256 256-256s256 114.6 256 256c0 92.5-49.4 176.3-128.1 221.8z" />
    </svg>
  );
}
