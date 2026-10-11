import { useState } from "react";
import { Plus } from "lucide-react";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { buttonVariants } from "@/components/ui/button";
import { KIND_ICON } from "@/components/automations/StepNode";
import { ACTION_KINDS, CONDITION_KINDS, KIND_LABEL } from "@/lib/automations";
import type { ActionKind, ConditionKind } from "@/lib/automations";
import { cn } from "@/lib/utils";

export interface PaletteProps {
  onAddCondition: (kind: ConditionKind) => void;
  onAddAction: (kind: ActionKind) => void;
}

/** Where steps come from: a condition goes after the others, before
    the actions; an action goes last. */
export function Palette({ onAddCondition, onAddAction }: PaletteProps) {
  return (
    <div className="flex gap-2">
      <Adder label="Add condition" kinds={CONDITION_KINDS} onPick={onAddCondition} />
      <Adder label="Add action" kinds={ACTION_KINDS} onPick={onAddAction} />
    </div>
  );
}

function Adder<K extends ConditionKind | ActionKind>({
  label,
  kinds,
  onPick,
}: {
  label: string;
  kinds: K[];
  onPick: (kind: K) => void;
}) {
  const [open, setOpen] = useState(false);
  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger className={cn(buttonVariants({ variant: "outline", size: "sm" }))}>
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
            {KIND_LABEL[kind]}
          </button>
        ))}
      </PopoverContent>
    </Popover>
  );
}
