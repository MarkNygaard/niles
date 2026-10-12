import { useState } from "react";
import { Plus } from "lucide-react";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { buttonVariants } from "@/components/ui/button";
import { KIND_ICON } from "@/components/automations/StepNode";
import { ACTION_KINDS, CONDITION_KINDS, KIND_LABEL, TRIGGER_KINDS } from "@/lib/automations";
import type { ActionKind, ConditionKind, StepKind, TriggerKind } from "@/lib/automations";
import { cn } from "@/lib/utils";

export interface PaletteProps {
  onAddTrigger: (kind: TriggerKind) => void;
  onAddCondition: (kind: ConditionKind) => void;
  onAddAction: (kind: ActionKind) => void;
}

/** Where steps come from: a trigger goes beside the others, a
    condition after the others, an action last. A step inside an `if`
    is added from that `if`'s panel. */
export function Palette({ onAddTrigger, onAddCondition, onAddAction }: PaletteProps) {
  return (
    <div className="flex flex-wrap gap-2">
      <Adder
        label="Add trigger"
        kinds={TRIGGER_KINDS}
        labels={KIND_LABEL.trigger}
        onPick={onAddTrigger}
      />
      <Adder
        label="Add condition"
        kinds={CONDITION_KINDS}
        labels={KIND_LABEL.condition}
        onPick={onAddCondition}
      />
      <Adder
        label="Add action"
        kinds={ACTION_KINDS}
        labels={KIND_LABEL.action}
        onPick={onAddAction}
      />
    </div>
  );
}

/** A button that opens a list of kinds to add one of. */
export function Adder<K extends StepKind>({
  label,
  kinds,
  labels,
  onPick,
  className,
}: {
  label: string;
  kinds: K[];
  labels: Record<K, string>;
  onPick: (kind: K) => void;
  className?: string;
}) {
  const [open, setOpen] = useState(false);
  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger className={cn(buttonVariants({ variant: "outline", size: "sm" }), className)}>
        <Plus />
        {label}
      </PopoverTrigger>
      <PopoverContent align="start" className="flex w-56 flex-col p-1">
        {kinds.map((kind) => (
          <button
            key={kind}
            type="button"
            onClick={() => {
              setOpen(false);
              onPick(kind);
            }}
            className={cn(
              "flex items-center gap-3 rounded-md px-2 py-2 text-left text-sm transition-colors",
              "hover:bg-muted focus-visible:ring-ring/50 focus-visible:ring-2 focus-visible:outline-none",
              "[&>svg]:text-muted-foreground [&>svg]:size-4",
            )}
          >
            {KIND_ICON[kind]}
            {labels[kind]}
          </button>
        ))}
      </PopoverContent>
    </Popover>
  );
}
