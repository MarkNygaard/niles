import { Power } from "lucide-react";
import { cn } from "@/lib/utils";

export interface PowerButtonProps {
  /** `null` when the device has never reported — not the same as off. */
  on: boolean | null;
  label: string;
  size?: "default" | "lg";
  disabled?: boolean;
  className?: string;
  onToggle: (next: boolean) => void;
}

/**
 * On, off, and the honest third state.
 *
 * A light Niles has never heard from is drawn dashed rather than off,
 * because showing it as off is a claim about a lamp that might well be
 * lit. Pressing it asks for on: that is what someone pressing a light
 * they can't read wants.
 */
export function PowerButton({
  on,
  label,
  size = "default",
  disabled,
  className,
  onToggle,
}: PowerButtonProps) {
  const unknown = on === null;
  return (
    <button
      type="button"
      disabled={disabled}
      aria-pressed={on ?? false}
      aria-label={`${label} — ${unknown ? "state unknown" : on ? "on" : "off"}`}
      onClick={() => onToggle(!on)}
      className={cn(
        "flex shrink-0 items-center justify-center rounded-full border transition-colors",
        "focus-visible:ring-3 focus-visible:ring-ring/50 focus-visible:outline-none",
        "disabled:cursor-not-allowed disabled:opacity-50",
        size === "lg" ? "size-12 [&_svg]:size-5" : "size-10 [&_svg]:size-4",
        on
          ? "border-transparent bg-amber-200 text-amber-950 hover:bg-amber-100"
          : unknown
            ? "border-dashed border-border text-muted-foreground hover:bg-muted"
            : "border-border text-muted-foreground hover:bg-muted hover:text-foreground",
        className,
      )}
    >
      <Power aria-hidden />
    </button>
  );
}
