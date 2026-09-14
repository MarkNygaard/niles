import { cn } from "@/lib/utils";

/**
 * Brand marks, as single paths on a 24-square.
 *
 * Inlined rather than pulled from a package: three glyphs is less than
 * a dependency, and an integration list that fetches logos from
 * somewhere else is a list that renders blank when that somewhere is
 * unreachable — which for a house is most of the reasons you would be
 * looking at this page.
 *
 * Paths from Simple Icons (CC0). The marks themselves remain their
 * owners' trademarks; they are here to identify the service, which is
 * what a trademark is for.
 */
const MARKS: Record<string, string> = {
  tado: "M22.486 7.795a1.514 1.514 0 1 0 0 3.029 1.514 1.514 0 0 0 0-3.029zm-8.504.003v2.456c-.457-.344-.945-.563-1.686-.563-1.814 0-2.833 1.364-2.833 3.267 0 1.792 1.019 3.247 2.833 3.247 1.781 0 2.817-1.46 2.82-3.247v-5.16zM1.89 7.799l-1.124.378V9.69H0v.945h.757v3.873c0 .84.67 1.51 1.518 1.51h1.128v-.943h-.946a.566.566 0 0 1-.568-.566v-3.874h3.215V9.69H1.89zm20.596.375a1.135 1.135 0 1 1 0 2.27 1.135 1.135 0 0 1 0-2.27zM5.48 9.69v.946h1.906c.354 0 .549.277.549.54v.773l-1.322-.001c-1.134 0-2.267.769-2.267 2.08 0 1.307 1.13 2.087 2.265 2.087.953 0 1.326-.57 1.326-.57v.47H9.07v-4.864c0-.784-.667-1.461-1.51-1.461zm12.861.002c-1.808 0-2.835 1.369-2.835 3.237 0 1.911 1.027 3.276 2.835 3.276 1.787 0 2.828-1.36 2.828-3.276 0-1.863-1.046-3.237-2.828-3.237z",
  linear:
    "M2.886 4.18A11.982 11.982 0 0 1 11.99 0C18.624 0 24 5.376 24 12.009c0 3.64-1.62 6.903-4.18 9.105L2.887 4.18ZM1.817 5.626l16.556 16.556c-.524.33-1.075.62-1.65.866L.951 7.277c.247-.575.537-1.126.866-1.65ZM.322 9.163l14.515 14.515c-.71.172-1.443.282-2.195.322L0 11.358a12 12 0 0 1 .322-2.195Zm-.17 4.862 9.823 9.824a12.02 12.02 0 0 1-9.824-9.824Z",
  github:
    "M12 .297c-6.63 0-12 5.373-12 12 0 5.303 3.438 9.8 8.205 11.385.6.113.82-.258.82-.577 0-.285-.01-1.04-.015-2.04-3.338.724-4.042-1.61-4.042-1.61C4.422 18.07 3.633 17.7 3.633 17.7c-1.087-.744.084-.729.084-.729 1.205.084 1.838 1.236 1.838 1.236 1.07 1.835 2.809 1.305 3.495.998.108-.776.417-1.305.76-1.605-2.665-.3-5.466-1.332-5.466-5.93 0-1.31.465-2.38 1.235-3.22-.135-.303-.54-1.523.105-3.176 0 0 1.005-.322 3.3 1.23.96-.267 1.98-.399 3-.405 1.02.006 2.04.138 3 .405 2.28-1.552 3.285-1.23 3.285-1.23.645 1.653.24 2.873.12 3.176.765.84 1.23 1.91 1.23 3.22 0 4.61-2.805 5.625-5.475 5.92.42.36.81 1.096.81 2.22 0 1.606-.015 2.896-.015 3.286 0 .315.21.69.825.57C20.565 22.092 24 17.592 24 12.297c0-6.627-5.373-12-12-12",
};

export interface BrandMarkProps {
  id: string;
  label: string;
  className?: string;
}

/**
 * The logo, or a letter when there is no logo to use.
 *
 * Not every service has a mark in a public set — Groq does not — and
 * drawing one that looks like theirs would be inventing a logo rather
 * than showing one. A letter is plainly a placeholder, which is the
 * honest thing for it to be.
 */
export function BrandMark({ id, label, className }: BrandMarkProps) {
  const path = MARKS[id];
  if (!path) {
    return (
      <span
        aria-hidden
        className={cn(
          "bg-muted text-muted-foreground flex size-5 shrink-0 items-center justify-center rounded-full text-[10px] font-semibold",
          className,
        )}
      >
        {label.slice(0, 1).toUpperCase()}
      </span>
    );
  }
  return (
    <svg
      aria-hidden
      viewBox="0 0 24 24"
      fill="currentColor"
      className={cn("size-5 shrink-0", className)}
    >
      <path d={path} />
    </svg>
  );
}
