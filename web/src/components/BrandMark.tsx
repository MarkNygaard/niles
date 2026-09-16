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
 * Paths from Simple Icons (CC0), except Groq's, which is in no public
 * set and comes from their own favicon. The marks remain their owners'
 * trademarks; they are here to identify the service, which is what a
 * trademark is for.
 *
 * Copy a path whole. The one that was here for tado stopped two thirds
 * of the way through, which drops the counters out of the letters and
 * draws the wordmark as four solid blobs.
 */
const MARKS: Record<string, string> = {
  tado: "M22.486 7.795a1.514 1.514 0 1 0 0 3.029 1.514 1.514 0 0 0 0-3.029zm-8.504.003v2.456c-.457-.344-.945-.563-1.686-.563-1.814 0-2.833 1.364-2.833 3.267 0 1.792 1.019 3.247 2.833 3.247 1.781 0 2.817-1.46 2.82-3.247v-5.16zM1.89 7.799l-1.124.378V9.69H0v.945h.757v3.873c0 .84.67 1.51 1.518 1.51h1.128v-.943h-.946a.566.566 0 0 1-.568-.566v-3.874h3.215V9.69H1.89zm20.596.375a1.135 1.135 0 1 1 0 2.27 1.135 1.135 0 0 1 0-2.27zM5.48 9.69v.946h1.906c.354 0 .549.277.549.54v.773l-1.322-.001c-1.134 0-2.267.769-2.267 2.08 0 1.307 1.13 2.087 2.265 2.087.953 0 1.326-.57 1.326-.57v.47H9.07v-4.864c0-.784-.667-1.461-1.51-1.461zm12.861.002c-1.808 0-2.835 1.369-2.835 3.237 0 1.911 1.027 3.276 2.835 3.276 1.787 0 2.828-1.36 2.828-3.276 0-1.863-1.046-3.237-2.828-3.237zm-6.046.95c1.14 0 1.68 1.185 1.68 2.316 0 1.117-.55 2.305-1.68 2.305-1.232 0-1.697-1.188-1.697-2.305 0-1.13.56-2.316 1.697-2.316zm6.046.005c1.12 0 1.703 1.18 1.703 2.3 0 1.117-.572 2.313-1.703 2.313-1.126 0-1.707-1.165-1.707-2.307 0-1.126.57-2.306 1.707-2.306zM6.614 12.9h1.322v1.207c0 .5-.373 1.062-1.323 1.062-.367 0-1.133-.19-1.133-1.134 0-.842.758-1.135 1.134-1.135Z",
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
 * Groq's mark, which has its own colours.
 *
 * Their favicon as they publish it — a silhouette would be a different
 * logo, not a smaller one. It is why this file cannot simply be a
 * table of paths: one of the four is a picture.
 */
function GroqMark({ className }: { className?: string }) {
  return (
    <svg
      aria-hidden
      viewBox="0 0 33 33"
      className={cn("size-5 shrink-0", className)}
    >
      <path fill="#F43E01" d="M.54.39h32v32h-32z" />
      <path
        fill="#fff"
        d="m18.445 4.406-9.468 13.74 7.341.665-1.69 9.578 9.469-13.74-7.342-.664 1.69-9.579Z"
      />
    </svg>
  );
}

/**
 * The logo, or a letter when there is no logo to use.
 *
 * The letter is still here for a service nobody has a mark for, and it
 * is plainly a placeholder — which is the honest thing for it to be.
 * Drawing something that merely looks like a company's logo would be
 * inventing one rather than showing it.
 */
export function BrandMark({ id, label, className }: BrandMarkProps) {
  if (id === "groq") return <GroqMark className={className} />;
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
