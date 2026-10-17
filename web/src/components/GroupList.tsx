import type { ReactNode } from "react";

/** One group: its name above, its rows in one rounded block. */
export function GroupList({
  id,
  label,
  footer,
  children,
}: {
  id: string;
  label: string;
  footer?: ReactNode;
  children: ReactNode;
}) {
  return (
    <section aria-labelledby={id} className="flex flex-col gap-1.5">
      <h3
        id={id}
        className="text-muted-foreground px-3 text-xs font-medium tracking-wide uppercase"
      >
        {label}
      </h3>
      <div className="bg-muted/50 divide-border/60 divide-y overflow-hidden rounded-2xl">
        {children}
      </div>
      {footer}
    </section>
  );
}
