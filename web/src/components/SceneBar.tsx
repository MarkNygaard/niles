import { useState } from "react";
import { Sparkles } from "lucide-react";
import { cn } from "@/lib/utils";
import { humanize } from "@/lib/rooms";

export interface SceneBarProps {
  scenes: string[];
  onApply: (name: string) => void;
}

/**
 * The saved scenes, as one press each.
 *
 * Above the rooms because that is the order you want them in: a scene
 * is the whole house at once, and reaching it by opening a room and
 * setting four lights is the work it exists to replace.
 *
 * Nothing at all when there are none. An empty shelf labelled Scenes
 * teaches somebody that the feature is missing, when what is missing is
 * that they have not saved one — and the way to save one is to say so,
 * which no button here could explain in the space it has.
 */
export function SceneBar({ scenes, onApply }: SceneBarProps) {
  const [applied, setApplied] = useState<string | null>(null);

  if (scenes.length === 0) return null;

  return (
    <div className="flex flex-wrap items-center gap-2" aria-label="Scenes">
      {scenes.map((name) => (
        <button
          key={name}
          type="button"
          onClick={() => {
            setApplied(name);
            onApply(name);
          }}
          className={cn(
            "bg-card flex items-center gap-2 rounded-full py-1.5 pr-4 pl-3 text-sm transition-colors",
            "hover:bg-muted focus-visible:ring-3 focus-visible:ring-ring/50 focus-visible:outline-none",
            // A scene has no state to report — it was applied, and what
            // happens to the lights afterwards is theirs. So the mark
            // is the press, not a toggle that would go on lying the
            // moment somebody dimmed one of them.
            applied === name && "ring-ring ring-1",
          )}
        >
          <Sparkles aria-hidden className="text-muted-foreground size-4" />
          {humanize(name)}
        </button>
      ))}
    </div>
  );
}
