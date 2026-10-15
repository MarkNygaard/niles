import { ChevronDown, Mail, ShieldQuestion } from "lucide-react";
import { BrandMark, hasBrandMark } from "@/components/BrandMark";
import { Button } from "@/components/ui/button";
import type { ApprovalCard as Card, Decision } from "@/lib/api";
import { cn } from "@/lib/utils";

/** A value longer than this is folded behind a chevron. */
const FOLD_AT = 140;

export interface ApprovalCardProps {
  card: Card;
  /** A decision is on its way. */
  deciding?: boolean;
  onDecide: (decision: Decision) => void;
}

/**
 * Something Niles is holding for a yes: what it would do, drawn as what
 * it is rather than as the call behind it, and the buttons that decide.
 *
 * The model asked; the person answers here, and Niles does exactly what
 * the card shows.
 */
export function ApprovalCard({ card, deciding, onDecide }: ApprovalCardProps) {
  const pending = card.status === "pending";
  return (
    <section
      aria-label={card.title}
      className="bg-muted/60 ring-foreground/10 flex w-full max-w-xl flex-col gap-3 self-start rounded-2xl p-4 ring-1"
    >
      <div className="flex items-start gap-3">
        <Icon icon={card.icon} label={card.subtitle} />
        <div className="min-w-0">
          <h3 className="text-base leading-snug font-medium">{card.title}</h3>
          <p className="text-muted-foreground text-sm">{card.subtitle}</p>
        </div>
      </div>

      {(card.fields.length > 0 || card.body) && (
        <div className="bg-background/70 flex flex-col gap-2 rounded-xl px-3 py-2.5 text-sm">
          {card.fields.map((field) => (
            <Field key={field.label} label={field.label} value={field.value} />
          ))}
          {card.body && (
            <p
              className={cn(
                "whitespace-pre-wrap",
                card.fields.length > 0 && "border-border/60 mt-1 border-t pt-2",
              )}
            >
              {card.body}
            </p>
          )}
        </div>
      )}

      {pending ? (
        <div className="flex flex-wrap gap-2">
          <Button className="rounded-full px-5" disabled={deciding} onClick={() => onDecide("once")}>
            Allow once
          </Button>
          {card.allow_conversation && (
            <Button
              variant="secondary"
              className="rounded-full px-5"
              disabled={deciding}
              onClick={() => onDecide("conversation")}
            >
              Allow for this conversation
            </Button>
          )}
          <Button
            variant="secondary"
            className="rounded-full px-5"
            disabled={deciding}
            onClick={() => onDecide("deny")}
          >
            Deny
          </Button>
        </div>
      ) : (
        <p
          role="status"
          className={cn(
            "text-sm",
            card.status === "failed" ? "text-destructive" : "text-muted-foreground",
          )}
        >
          {outcome(card)}
        </p>
      )}
    </section>
  );
}

/** What became of it, in a line. */
export function outcome(card: Card): string {
  switch (card.status) {
    case "allowed":
      return "Allowed. " + (card.outcome && card.outcome !== "Done" ? card.outcome : "Done.");
    case "denied":
      return "Denied. Nothing was done.";
    case "lapsed":
      return "Lapsed. Nothing was done.";
    case "failed":
      return `Allowed, but it didn't work: ${card.outcome ?? "no reason given"}`;
    case "pending":
      return "Waiting for you.";
  }
}

function Icon({ icon, label }: { icon: string | null; label: string }) {
  const box = "bg-background flex size-10 shrink-0 items-center justify-center rounded-xl";
  if (icon && hasBrandMark(icon)) {
    return (
      <span className={box}>
        <BrandMark id={icon} label={label} className="size-6" />
      </span>
    );
  }
  const Glyph = icon === "mail" ? Mail : ShieldQuestion;
  return (
    <span className={box}>
      <Glyph aria-hidden className="size-5" />
    </span>
  );
}

function Field({ label, value }: { label: string; value: string }) {
  const row = "grid grid-cols-[minmax(5rem,auto)_1fr] gap-3";
  if (value.length <= FOLD_AT) {
    return (
      <div className={row}>
        <span className="text-muted-foreground">{label}</span>
        <span className="min-w-0 break-words">{value}</span>
      </div>
    );
  }
  return (
    <details className="group">
      <summary className={cn(row, "cursor-pointer list-none")}>
        <span className="text-muted-foreground">{label}</span>
        <span className="flex min-w-0 items-start justify-between gap-2">
          <span className="truncate">{value}</span>
          <ChevronDown
            aria-hidden
            className="text-muted-foreground size-4 shrink-0 transition-transform group-open:rotate-180"
          />
        </span>
      </summary>
      <p className="mt-1 whitespace-pre-wrap break-words">{value}</p>
    </details>
  );
}
