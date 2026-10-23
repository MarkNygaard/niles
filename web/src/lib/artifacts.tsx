import type { ReactNode } from "react";
import { FileText, Footprints, Luggage, Podcast } from "lucide-react";
import type { LucideIcon } from "lucide-react";
import { PodcastView } from "@/components/PodcastView";
import { TrainingPlanView } from "@/components/TrainingPlanView";
import { TripView } from "@/components/TripView";
import type { Artifact } from "@/lib/api";
import { Markdown, plainText, toggleCheck } from "@/lib/markdown";
import { podcastOf, podcastText } from "@/lib/podcast";
import { planOf, planText } from "@/lib/trainingPlan";
import { tripOf, tripText } from "@/lib/trip";

/** Every query about artifacts, to refetch them all at once. */
export const ARTIFACTS = ["artifacts"];
export const ARTIFACT_LIST = [...ARTIFACTS, "list"];
export const artifactKey = (id: number) => [...ARTIFACTS, "one", id];

export interface KindViewProps {
  content: unknown;
  /** A change the person made in the view: a ticked item. Saved as a
      version by them, with `reason`. Without it, the view is read-only. */
  onChange?: (content: unknown, reason: string) => void;
  busy?: boolean;
}

/**
 * A kind of artifact, as the app shows it: the twin of `KINDS` in
 * crates/niles-artifacts/src/kinds.rs. A new kind is one more entry
 * here, with its own view.
 */
export interface Kind {
  id: string;
  label: string;
  icon: LucideIcon;
  /** Before what its job is doing, in the panel's status line. */
  emoji: string;
  View: (props: KindViewProps) => ReactNode;
  /** As text, for Copy. */
  text: (content: unknown) => string;
}

/** A document's markdown; "" when it has none. */
export function markdownOf(content: unknown): string {
  if (content && typeof content === "object" && "markdown" in content) {
    const markdown = (content as { markdown: unknown }).markdown;
    if (typeof markdown === "string") return markdown;
  }
  return "";
}

function DocumentView({ content, onChange, busy }: KindViewProps) {
  const source = markdownOf(content);
  return (
    <Markdown
      source={source}
      disabled={busy}
      onToggle={
        onChange &&
        ((item, checked) =>
          onChange(
            { markdown: toggleCheck(source, item.line) },
            `${checked ? "Ticked" : "Unticked"} “${plainText(item.text)}”`,
          ))
      }
    />
  );
}

const DOCUMENT: Kind = {
  id: "document",
  label: "Document",
  icon: FileText,
  emoji: "✍️",
  View: DocumentView,
  text: markdownOf,
};

const TRAINING_PLAN: Kind = {
  id: "training_plan",
  label: "Training plan",
  icon: Footprints,
  emoji: "🏃",
  View: TrainingPlanView,
  text: (content) => {
    const plan = planOf(content);
    return plan ? planText(plan) : JSON.stringify(content, null, 2);
  },
};

const TRIP: Kind = {
  id: "trip",
  label: "Trip",
  icon: Luggage,
  emoji: "🧳",
  View: TripView,
  text: (content) => {
    const trip = tripOf(content);
    return trip ? tripText(trip) : JSON.stringify(content, null, 2);
  },
};

const PODCAST: Kind = {
  id: "podcast",
  label: "Podcast",
  icon: Podcast,
  emoji: "🎙️",
  View: PodcastView,
  text: (content) => {
    const podcast = podcastOf(content);
    return podcast ? podcastText(podcast) : JSON.stringify(content, null, 2);
  },
};

export const KINDS: Record<string, Kind> = {
  document: DOCUMENT,
  training_plan: TRAINING_PLAN,
  trip: TRIP,
  podcast: PODCAST,
};

/** The kind named `id`, or one that says this app cannot show it yet:
    a Niles newer than the app on the phone. */
export function kindOf(id: string): Kind {
  return (
    KINDS[id] ?? {
      ...DOCUMENT,
      id,
      label: "Saved",
      View: () => (
        <p className="text-muted-foreground text-sm">
          This app cannot show this yet. Reload it to get the newest one.
        </p>
      ),
      text: (content: unknown) => JSON.stringify(content, null, 2),
    }
  );
}

/** What the newest job is doing, with its kind's emoji: "✍️ Writing
    your Thailand travel plan…". Undefined when nothing is being written. */
export function workingLine(artifacts: Artifact[] | undefined): string | undefined {
  const working = (artifacts ?? [])
    .filter((a) => a.status === "working")
    .sort((a, b) => b.updated_at.localeCompare(a.updated_at))[0];
  if (!working) return undefined;
  return `${kindOf(working.kind).emoji} ${working.status_text ?? `Writing “${working.title}”…`}`;
}

/** Saved as the tab shows it: pinned first, then the rest by when they
    changed, then the archived ones. */
export function arrange(artifacts: Artifact[]): {
  pinned: Artifact[];
  recent: Artifact[];
  archived: Artifact[];
} {
  const newest = (a: Artifact, b: Artifact) => b.updated_at.localeCompare(a.updated_at);
  const shown = artifacts.filter((a) => !a.archived);
  return {
    pinned: shown.filter((a) => a.pinned).sort(newest),
    recent: shown.filter((a) => !a.pinned).sort(newest),
    archived: artifacts.filter((a) => a.archived).sort(newest),
  };
}
