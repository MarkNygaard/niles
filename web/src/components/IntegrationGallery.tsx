import { BrandMark, hasBrandMark } from "@/components/BrandMark";
import {
  AudioLines,
  Bot,
  Brain,
  Calendar,
  CalendarDays,
  Check,
  ChevronRight,
  ListTodo,
  Music,
  Network,
  Plug,
  ShoppingBasket,
  Sparkles,
  Speaker,
  Thermometer,
  Tv,
  Wifi,
  X,
  type LucideIcon,
} from "lucide-react";
import { Badge } from "@/components/ui/badge";
import {
  Dialog,
  DialogBody,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { cn } from "@/lib/utils";
import type { Importance, Integration, IntegrationGroup } from "@/lib/api";

/**
 * The icon names the catalogue uses. A table of icons rather than of
 * integrations: a new entry that reuses one needs nothing here, and one
 * that names an icon missing from it still gets a plug.
 */
const ICONS: Record<string, LucideIcon> = {
  network: Network,
  sparkles: Sparkles,
  "audio-lines": AudioLines,
  brain: Brain,
  speaker: Speaker,
  music: Music,
  tv: Tv,
  wifi: Wifi,
  thermometer: Thermometer,
  calendar: Calendar,
  "calendar-days": CalendarDays,
  "shopping-basket": ShoppingBasket,
  bot: Bot,
  "list-todo": ListTodo,
};

const ROLE_LABELS: Record<"stt" | "llm", string> = {
  stt: "Speech-to-text",
  llm: "AI",
};

const RANK: Record<Importance, number> = {
  required: 0,
  recommended: 1,
  optional: 2,
};

/**
 * The integrations under their groups, groups in the order they first
 * appear. The server sends them in page order, so this is the page's
 * order without the page knowing what the groups are.
 */
export function groupsOf(
  integrations: Integration[],
): { group: IntegrationGroup; items: Integration[] }[] {
  const groups = new Map<string, { group: IntegrationGroup; items: Integration[] }>();
  for (const integration of integrations) {
    const entry = groups.get(integration.group.id);
    if (entry) entry.items.push(integration);
    else groups.set(integration.group.id, { group: integration.group, items: [integration] });
  }
  return [...groups.values()];
}

/** The roles no added provider serves yet. */
export function uncoveredRoles(integrations: Integration[]): ("stt" | "llm")[] {
  const added = integrations.filter((i) => i.added && i.kind === "provider");
  return (["stt", "llm"] as const).filter(
    (role) => !added.some((p) => p.serves.includes(role)),
  );
}

/**
 * Whether its card says Required.
 *
 * A provider is required only as "one of" for a role: once something
 * serves that role, the others are a choice rather than a gap, and a
 * badge saying otherwise sends somebody off to make accounts they do
 * not need.
 */
export function showsRequired(
  integration: Integration,
  uncovered: ("stt" | "llm")[],
): boolean {
  if (integration.added || integration.importance !== "required") return false;
  if (integration.kind !== "provider") return true;
  return integration.serves.some((role) => uncovered.includes(role));
}

export interface IntegrationGalleryProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  integrations: Integration[];
  disabled?: boolean;
  /** An integration's card was picked: set it up, or open what is set up. */
  onPick: (integration: Integration) => void;
}

/**
 * Everything Niles can be connected to, as cards.
 *
 * Cards rather than rows because a newcomer needs more than a name to
 * choose: what it does for the house and what they need before they
 * start. Required first, so a new install works down from the top.
 */
export function IntegrationGallery({
  open,
  onOpenChange,
  integrations,
  disabled,
  onPick,
}: IntegrationGalleryProps) {
  const uncovered = uncoveredRoles(integrations);
  const everything = integrations.length > 0 && integrations.every((i) => i.added);

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:w-[calc(100vw-2rem)] sm:max-w-4xl">
        <DialogHeader>
          <div className="min-w-0">
            <DialogTitle>Add an integration</DialogTitle>
            <DialogDescription>
              Start with the ones marked Required: they are what Niles needs to
              reach your lights, hear you and answer. The rest add to the house
              and can come later.
            </DialogDescription>
          </div>
          <DialogClose
            aria-label="Close"
            className="text-muted-foreground hover:text-foreground focus-visible:ring-3 focus-visible:ring-ring/50 flex size-8 shrink-0 items-center justify-center rounded-lg focus-visible:outline-none"
          >
            <X aria-hidden className="size-4" />
          </DialogClose>
        </DialogHeader>
        <DialogBody className="flex flex-col gap-6 pb-6">
          {everything && (
            <p className="text-muted-foreground text-sm">
              Everything here is already set up.
            </p>
          )}
          {groupsOf(integrations).map(({ group, items }) => (
            <section
              key={group.id}
              aria-labelledby={`gallery-group-${group.id}`}
              className="flex flex-col gap-3"
            >
              <div>
                <h3 id={`gallery-group-${group.id}`} className="text-sm font-semibold">
                  {group.label}
                </h3>
                <p className="text-muted-foreground text-sm">{group.blurb}</p>
                {group.requirement && (
                  <Requirement text={group.requirement} items={items} uncovered={uncovered} />
                )}
              </div>
              <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-3">
                {[...items]
                  .sort((a, b) => RANK[a.importance] - RANK[b.importance])
                  .map((integration) => (
                    <GalleryCard
                      key={integration.id}
                      integration={integration}
                      required={showsRequired(integration, uncovered)}
                      disabled={disabled}
                      onPick={() => onPick(integration)}
                    />
                  ))}
              </div>
            </section>
          ))}
        </DialogBody>
      </DialogContent>
    </Dialog>
  );
}

/** The group's "one of these", and how far along it is. */
function Requirement({
  text,
  items,
  uncovered,
}: {
  text: string;
  items: Integration[];
  uncovered: ("stt" | "llm")[];
}) {
  const providers = items.filter((i) => i.kind === "provider");
  const met = uncovered.length === 0;
  return (
    <div
      className={cn(
        "mt-2 rounded-lg px-3 py-2 text-sm",
        met ? "bg-muted/60" : "bg-primary/10",
      )}
    >
      <p className="font-medium">{text}</p>
      {providers.length > 0 && (
        <ul className="text-muted-foreground mt-1 flex flex-wrap gap-x-4 gap-y-1 text-xs">
          {(["stt", "llm"] as const).map((role) => {
            const by = providers.filter((p) => p.added && p.serves.includes(role));
            return (
              <li key={role} className="flex items-center gap-1">
                {by.length > 0 && <Check aria-hidden className="text-lit size-3" />}
                {ROLE_LABELS[role]}:{" "}
                {by.length > 0 ? by.map((p) => p.label).join(", ") : "not yet"}
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}

function GalleryCard({
  integration,
  required,
  disabled,
  onPick,
}: {
  integration: Integration;
  required: boolean;
  disabled?: boolean;
  onPick: () => void;
}) {
  // The service's own mark, as the page shows it once added; the
  // catalogue's generic icon only where there is none (the broker).
  const brand = hasBrandMark(integration.id);
  const Icon = ICONS[integration.icon] ?? Plug;

  const body = (
    <>
      <span className="flex items-start gap-3">
        <span
          aria-hidden
          data-logo={brand ? "brand" : "icon"}
          className="bg-muted text-foreground flex size-9 shrink-0 items-center justify-center rounded-lg"
        >
          {brand ? (
            <BrandMark id={integration.id} label={integration.label} className="size-5" />
          ) : (
            <Icon className="size-5" />
          )}
        </span>
        <span className="min-w-0 flex-1">
          <span className="flex flex-wrap items-center gap-1.5">
            <span className="text-sm font-medium">{integration.label}</span>
            {integration.added ? (
              <Badge variant="outline">
                <Check aria-hidden /> Added
              </Badge>
            ) : required ? (
              <Badge>Required</Badge>
            ) : integration.importance === "recommended" ? (
              <Badge variant="secondary">Recommended</Badge>
            ) : null}
          </span>
          {integration.kind === "provider" && (
            <span className="text-muted-foreground mt-0.5 block text-xs">
              {integration.serves.map((r) => ROLE_LABELS[r]).join(" and ")}
            </span>
          )}
        </span>
      </span>
      <span className="block text-sm">{integration.blurb}</span>
      <span className="text-muted-foreground block text-xs">
        <span className="text-foreground font-medium">You need: </span>
        {integration.needs}
      </span>
      <span className="text-muted-foreground mt-auto flex items-center gap-1 pt-1 text-xs font-medium">
        {integration.added ? "Open its settings" : "Set up"}
        <ChevronRight aria-hidden className="size-3.5" />
      </span>
    </>
  );

  const shape =
    "border-border flex h-full flex-col gap-2 rounded-xl border p-3 text-left";

  return (
    <button
      type="button"
      disabled={disabled}
      onClick={onPick}
      className={cn(
        shape,
        "hover:bg-muted/60 focus-visible:ring-3 focus-visible:ring-ring/50 focus-visible:outline-none disabled:opacity-50",
      )}
    >
      {body}
    </button>
  );
}
