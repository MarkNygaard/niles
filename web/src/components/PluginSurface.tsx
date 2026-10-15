import { useEffect, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { SendHorizontal } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { Slider } from "@/components/ui/slider";
import { Switch } from "@/components/ui/switch";
import { SurfaceChart } from "@/components/SurfaceChart";
import { ApiError, api } from "@/lib/api";
import type { ActionValue, Surface, Tone, UiNode } from "@/lib/api";
import { imageUrl } from "@/lib/plugins";
import { cn } from "@/lib/utils";

/** The query a surface is kept under. A `ui_invalidated` frame for the
    plugin and surface refetches every room's copy that is on screen. */
export function surfaceKey(plugin: string, surface: string, room?: string | null) {
  return ["surface", plugin, surface, room ?? ""] as const;
}

/** Why a surface is not drawn, as it is shown in its place. */
export function failureOf(error: unknown): { text: string; detail: string | null } {
  // 422 is the plugin's own sentence ("No prices yet."), meant for
  // whoever is looking. Anything else is the plugin not being there,
  // which is said calmly, with what Niles knows beneath it.
  if (error instanceof ApiError && error.status === 422) {
    return { text: error.message, detail: null };
  }
  return {
    text: "Not available right now",
    detail: error instanceof ApiError && error.status === 503 ? error.message : null,
  };
}

/** What a Choice offers, in its order. */
export function choiceItems(node: Extract<UiNode, { type: "choice" }>) {
  return node.options.map((o) => ({ value: o.value, label: o.label }));
}

const TONES: Record<Tone, string> = {
  neutral: "bg-secondary text-secondary-foreground",
  good: "bg-ok/15 text-ok",
  warning: "bg-warn/20 text-foreground",
  bad: "bg-destructive/10 text-destructive dark:bg-destructive/20",
};

export interface PluginSurfaceProps {
  plugin: string;
  surface: string;
  /** The room whose card a widget is on, or its home room. */
  room?: string | null;
  /** Seconds between renders while it is on screen, from the manifest. */
  refreshEvery?: number | null;
}

/**
 * A plugin's page or widget, drawn from the tree it describes with the
 * app's own components. Nothing of the plugin's runs here: the tree is
 * data, and every node is one of a fixed few.
 *
 * Rendered when it comes on screen, every `refreshEvery` seconds while
 * it stays there, when the plugin says it changed (a `ui_invalidated`
 * frame on the event stream; see `useDeviceStream`), and after an
 * action that asks for it.
 */
export function PluginSurface({ plugin, surface, room, refreshEvery }: PluginSurfaceProps) {
  const queryClient = useQueryClient();
  const key = surfaceKey(plugin, surface, room);
  const drawn = useQuery({
    queryKey: key,
    queryFn: () => api.surface(plugin, surface, room),
    refetchInterval: refreshEvery ? refreshEvery * 1000 : false,
    retry: false,
  });
  const [toast, setToast] = useState<{ text: string; error: boolean } | null>(null);

  useEffect(() => {
    if (!toast) return;
    const timer = setTimeout(() => setToast(null), 4_000);
    return () => clearTimeout(timer);
  }, [toast]);

  /** Use a control. False when the plugin refused, so the control can
      put back what it showed. */
  async function act(control: string, value: ActionValue): Promise<boolean> {
    try {
      const outcome = await api.act(plugin, surface, control, value, room);
      if (outcome.toast) setToast({ text: outcome.toast, error: false });
      if (outcome.rerender) await queryClient.invalidateQueries({ queryKey: key });
      return true;
    } catch (e) {
      setToast({ text: e instanceof Error ? e.message : String(e), error: true });
      return false;
    }
  }

  if (drawn.isPending) return <Skeleton className="h-24 w-full" />;
  if (drawn.error || !drawn.data) {
    const { text, detail } = failureOf(drawn.error);
    return (
      <div data-surface="unavailable" className="flex flex-col gap-1 py-2 text-sm">
        <p className="text-muted-foreground">{text}</p>
        {detail && <p className="text-muted-foreground/80 text-xs">{detail}</p>}
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-3">
      {/* Keyed by the render, so a control shows what the plugin drew
          again rather than what was last pressed. */}
      <SurfaceNode
        key={drawn.dataUpdatedAt}
        tree={drawn.data}
        index={0}
        plugin={plugin}
        act={act}
      />
      {toast && (
        <p
          role={toast.error ? "alert" : "status"}
          className={cn(
            "rounded-lg px-3 py-2 text-sm",
            toast.error ? "bg-destructive/10 text-destructive" : "bg-muted",
          )}
        >
          {toast.text}
        </p>
      )}
    </div>
  );
}

interface NodeProps {
  tree: Surface;
  index: number;
  plugin: string;
  act: (control: string, value: ActionValue) => Promise<boolean>;
}

function SurfaceNode({ tree, index, plugin, act }: NodeProps) {
  const node = tree.nodes[index];
  if (!node) return null;
  const children = node.children.map((child) => (
    <SurfaceNode key={child} tree={tree} index={child} plugin={plugin} act={act} />
  ));
  switch (node.type) {
    case "stack":
      return <div className="flex flex-col gap-3">{children}</div>;
    case "row":
      // Side by side from `sm`; on a phone a row is a stack.
      return (
        <div className="flex flex-col gap-3 sm:flex-row sm:flex-wrap sm:items-start [&>*]:min-w-0 sm:[&>*]:flex-1">
          {children}
        </div>
      );
    case "card":
      return (
        <Card size="sm">
          {node.title && (
            <CardHeader>
              <CardTitle>{node.title}</CardTitle>
            </CardHeader>
          )}
          <CardContent className="flex flex-col gap-3">{children}</CardContent>
        </Card>
      );
    case "heading":
      return <h3 className="font-heading text-base font-semibold">{node.text}</h3>;
    case "text":
      return (
        <p className={node.muted ? "text-muted-foreground text-xs" : "text-sm"}>{node.value}</p>
      );
    case "reading":
      return (
        <div className="flex flex-col">
          <span className="text-muted-foreground text-xs">{node.label}</span>
          <span className="text-3xl font-semibold tracking-tight tabular-nums">
            {node.value}
            {node.unit && (
              <span className="text-muted-foreground ml-1 text-base font-normal">
                {node.unit}
              </span>
            )}
          </span>
        </div>
      );
    case "badge":
      return (
        <Badge data-tone={node.tone} className={cn("self-start", TONES[node.tone])}>
          {node.label}
        </Badge>
      );
    case "items":
      return (
        <ul className="divide-border divide-y">
          {node.items.map((item, n) => (
            <li key={n} className="flex items-baseline gap-3 py-2 text-sm">
              <span className="min-w-0 flex-1">
                <span className="block truncate">{item.title}</span>
                {item.detail && (
                  <span className="text-muted-foreground block text-xs">{item.detail}</span>
                )}
              </span>
              {item.trailing && (
                <span className="text-muted-foreground shrink-0 tabular-nums">
                  {item.trailing}
                </span>
              )}
            </li>
          ))}
        </ul>
      );
    case "chart":
      return (
        <SurfaceChart kind={node.kind} labels={node.labels} series={node.series} unit={node.unit} />
      );
    case "image":
      return (
        <img
          src={imageUrl(plugin, node.src)}
          alt={node.alt}
          loading="lazy"
          referrerPolicy="no-referrer"
          className="h-auto max-w-full self-start rounded-lg"
        />
      );
    case "button":
      return <ActionButton node={node} act={act} />;
    case "switch":
      return <SwitchControl node={node} act={act} />;
    case "slider":
      return <SliderControl node={node} act={act} />;
    case "choice":
      return <ChoiceControl node={node} act={act} />;
    case "text_field":
      return <TextControl node={node} act={act} />;
    default:
      return null;
  }
}

type Act = NodeProps["act"];

function ActionButton({ node, act }: { node: Extract<UiNode, { type: "button" }>; act: Act }) {
  const [busy, setBusy] = useState(false);
  return (
    <div>
      <Button
        variant={
          node.style === "primary"
            ? "default"
            : node.style === "destructive"
              ? "destructive"
              : "outline"
        }
        disabled={busy}
        onClick={async () => {
          setBusy(true);
          await act(node.id, { type: "pressed" });
          setBusy(false);
        }}
      >
        {node.label}
      </Button>
    </div>
  );
}

/** A control's value: what was drawn, the new one at once when moved,
    and the drawn one back if the plugin refused it. */
function useControl<T>(drawn: T, id: string, act: Act, wrap: (value: T) => ActionValue) {
  const [value, setValue] = useState(drawn);
  const [busy, setBusy] = useState(false);
  async function change(next: T) {
    const before = value;
    setValue(next);
    setBusy(true);
    if (!(await act(id, wrap(next)))) setValue(before);
    setBusy(false);
  }
  return { value, setValue, busy, change };
}

function SwitchControl({ node, act }: { node: Extract<UiNode, { type: "switch" }>; act: Act }) {
  const control = useControl(node.value, node.id, act, (value) => ({ type: "toggled", value }));
  return (
    <div className="flex min-h-10 items-center justify-between gap-3 text-sm">
      <span>{node.label}</span>
      <Switch
        aria-label={node.label}
        checked={control.value}
        disabled={control.busy}
        onCheckedChange={(on: boolean) => control.change(on)}
      />
    </div>
  );
}

function SliderControl({ node, act }: { node: Extract<UiNode, { type: "slider" }>; act: Act }) {
  const control = useControl(node.value, node.id, act, (value) => ({ type: "number", value }));
  const text = `${control.value}${node.unit ? ` ${node.unit}` : ""}`;
  return (
    <div className="flex flex-col gap-2 text-sm">
      <div className="flex items-baseline justify-between gap-3">
        <span>{node.label}</span>
        <span className="text-muted-foreground tabular-nums">{text}</span>
      </div>
      <Slider
        value={control.value}
        min={node.min}
        max={node.max}
        step={node.step}
        disabled={control.busy}
        thumbLabel={node.label}
        thumbValueText={text}
        onValueChange={(next: number, details: { reason: string }) => {
          control.setValue(next);
          // A tap on the track never reaches `onValueCommitted` on a
          // touch screen; see LightRow.
          if (details.reason === "track-press") control.change(next);
        }}
        onValueCommitted={(next: number) => control.change(next)}
      />
    </div>
  );
}

function ChoiceControl({ node, act }: { node: Extract<UiNode, { type: "choice" }>; act: Act }) {
  const control = useControl(node.value, node.id, act, (value) => ({
    type: "chosen",
    value: value ?? "",
  }));
  const items = choiceItems(node);
  return (
    <div className="flex flex-col gap-1.5 text-sm">
      <span>{node.label}</span>
      <Select
        items={items}
        value={control.value}
        disabled={control.busy}
        onValueChange={(next: string | null) => {
          if (next !== null) control.change(next);
        }}
      >
        <SelectTrigger aria-label={node.label} className="w-full sm:w-64">
          <SelectValue placeholder="Choose…" />
        </SelectTrigger>
        <SelectContent>
          {items.map((option) => (
            <SelectItem key={option.value} value={option.value}>
              {option.label}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
    </div>
  );
}

function TextControl({ node, act }: { node: Extract<UiNode, { type: "text_field" }>; act: Act }) {
  const [draft, setDraft] = useState(node.value ?? "");
  const [busy, setBusy] = useState(false);
  async function submit() {
    setBusy(true);
    await act(node.id, { type: "text", value: draft });
    setBusy(false);
  }
  return (
    <form
      className="flex flex-col gap-1.5 text-sm"
      onSubmit={(e) => {
        e.preventDefault();
        submit();
      }}
    >
      <span>{node.label}</span>
      <div className="flex gap-2">
        <Input
          aria-label={node.label}
          autoComplete="off"
          placeholder={node.placeholder ?? undefined}
          value={draft}
          disabled={busy}
          onChange={(e) => setDraft(e.target.value)}
        />
        <Button type="submit" variant="outline" disabled={busy} aria-label={`Send ${node.label}`}>
          <SendHorizontal aria-hidden />
        </Button>
      </div>
    </form>
  );
}
