import { Handle, Position } from "@xyflow/react";
import type { Node, NodeProps } from "@xyflow/react";
import {
  AlarmClock,
  BellRing,
  Blinds,
  Clock,
  DoorClosed,
  DoorOpen,
  GitBranch,
  Hourglass,
  House,
  Lamp,
  LampCeiling,
  Lightbulb,
  ListFilter,
  PersonStanding,
  Power,
  Sunrise,
  Sunset,
  Thermometer,
  Timer,
  ToggleLeft,
  X,
} from "lucide-react";
import { kindOf, labelOf } from "@/lib/automations";
import type { StepData, StepKind } from "@/lib/automations";
import { cn } from "@/lib/utils";

export const KIND_ICON: Record<StepKind, React.ReactNode> = {
  contact: <DoorOpen />,
  motion: <PersonStanding />,
  device_state: <Power />,
  device_action: <ToggleLeft />,
  timer_fired: <Timer />,
  presence: <House />,
  numeric: <Thermometer />,
  time: <AlarmClock />,
  sun: <Sunset />,
  time_of_day: <Clock />,
  device_is: <Lightbulb />,
  contact_is: <DoorClosed />,
  motion_is: <PersonStanding />,
  presence_is: <House />,
  triggered_by: <ListFilter />,
  set_room: <LampCeiling />,
  set_device: <Lamp />,
  set_blinds: <Blinds />,
  wake_up: <Sunrise />,
  notify: <BellRing />,
  delay: <Hourglass />,
  if: <GitBranch />,
};

/** A step as the canvas draws it: removable, unless it is the only
    trigger or the flow is only being looked at. */
export type CanvasNode = Node<StepData & { onRemove?: () => void }, "step">;

const HANDLE = "!h-2 !w-2 !border-0 !bg-border";

export function StepNode({ data, selected }: NodeProps<CanvasNode>) {
  const { step, heading, summary, onRemove } = data;
  const kind = kindOf(step);
  return (
    <div
      className={cn(
        "bg-card text-card-foreground flex h-full w-full items-center gap-3 rounded-xl border px-3 shadow-sm transition-colors",
        selected ? "border-primary ring-primary/30 ring-2" : "border-border",
      )}
    >
      {step.role !== "trigger" && (
        <Handle type="target" position={Position.Top} isConnectable={false} className={HANDLE} />
      )}
      <span className="bg-muted text-muted-foreground flex size-9 shrink-0 items-center justify-center rounded-lg [&>svg]:size-4">
        {KIND_ICON[kind]}
      </span>
      <div className="min-w-0 flex-1">
        <div className="text-muted-foreground truncate text-[11px] font-medium tracking-wide uppercase">
          {heading} · {labelOf(step)}
        </div>
        <div className="truncate text-sm font-medium" title={summary}>
          {summary}
        </div>
      </div>
      {onRemove && (
        <button
          type="button"
          aria-label={`Remove ${summary}`}
          // `nodrag` keeps React Flow from reading the press as the
          // start of a pan; stopping it keeps it from selecting.
          className="nodrag text-muted-foreground hover:text-foreground focus-visible:ring-ring/50 -mr-1 flex size-7 shrink-0 items-center justify-center rounded-md focus-visible:ring-3 focus-visible:outline-none"
          onClick={(e) => {
            e.stopPropagation();
            onRemove();
          }}
        >
          <X className="size-4" />
        </button>
      )}
      <Handle type="source" position={Position.Bottom} isConnectable={false} className={HANDLE} />
    </div>
  );
}

/** Module-level, so React Flow sees the same object every render and
    does not warn or remount every node. */
export const nodeTypes = { step: StepNode };
