import { useState } from "react";
import { Plus, Trash2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { cn } from "@/lib/utils";
import type { WledStrip } from "@/lib/api";

export interface WledCardProps {
  strips: WledStrip[];
  saving?: boolean;
  error?: string;
  onChange: (strips: WledStrip[]) => void;
}

/**
 * What each WLED strip is made of.
 *
 * Niles has to be told, because WLED never says. Its MQTT feed carries
 * a brightness and a colour and nothing else — no equivalent of Z2M's
 * `bridge/devices`, and no way to ask. What a strip is currently
 * showing is not an answer either: an analog white strip publishes a
 * colour anyway, and a colour strip in white mode publishes none.
 *
 * Getting it wrong is quiet in both directions. A white-only strip
 * offered a colour wheel takes the command and ignores it; a colour
 * strip never told it has white channels sits at one temperature all
 * evening while the rest of the house warms up.
 */
const CHANNELS = [
  {
    id: "rgb",
    label: "Colour",
    hint: "RGB only — the usual sort",
    rgb: true,
    white_balance: false,
  },
  {
    id: "cct",
    label: "White balance",
    hint: "Warm and cold white, no colour",
    rgb: false,
    white_balance: true,
  },
  {
    id: "both",
    label: "Both",
    hint: "Colour and white channels",
    rgb: true,
    white_balance: true,
  },
] as const;

/** Which of the three a strip is, from the two flags it stores. */
function channelsOf(strip: WledStrip): string {
  const rgb = strip.rgb ?? true;
  const white = strip.white_balance ?? false;
  if (rgb && white) return "both";
  return white ? "cct" : "rgb";
}

export function WledCard({ strips, saving, error, onChange }: WledCardProps) {
  const [name, setName] = useState("");
  const [topic, setTopic] = useState("");
  const [problem, setProblem] = useState<string | null>(null);

  function add(event: React.FormEvent) {
    event.preventDefault();
    const strip = name.trim().toLowerCase();
    const base = topic.trim();
    if (!strip || !base) return;
    // The server checks all three. Saying it here means the answer
    // arrives while the thing being described is still on screen.
    if (!/^[a-z0-9_]+\/[a-z0-9_]+$/.test(strip)) {
      setProblem("A name is a room and a device, like living_room/ceiling.");
      return;
    }
    if (strips.some((s) => s.name === strip)) {
      setProblem(`There is already a strip called ${strip}.`);
      return;
    }
    setProblem(null);
    onChange([
      ...strips,
      { name: strip, topic: base, rgb: true, white_balance: false },
    ]);
    setName("");
    setTopic("");
  }

  function setChannels(strip: WledStrip, id: string) {
    const choice = CHANNELS.find((c) => c.id === id);
    if (!choice) return;
    onChange(
      strips.map((s) =>
        s.name === strip.name
          ? { ...s, rgb: choice.rgb, white_balance: choice.white_balance }
          : s,
      ),
    );
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle>WLED strips</CardTitle>
        <CardDescription>
          What each strip is made of. WLED never says — its feed carries a
          brightness and a colour and nothing else — so the curve warms the
          white channels of the strips named here and leaves the rest their
          colour. Changing this takes a restart.
        </CardDescription>
      </CardHeader>
      <CardContent className="flex flex-col gap-4">
        {strips.length === 0 && (
          <p className="text-muted-foreground text-sm">
            None yet. Add one with the topic it publishes on — the same one
            you set under Sync Interfaces in WLED itself.
          </p>
        )}

        {strips.map((strip) => {
          const current = channelsOf(strip);
          return (
            <div
              key={strip.name}
              className="bg-muted/40 flex flex-col gap-3 rounded-lg px-3 py-2.5"
            >
              <div className="flex items-center gap-3">
                <span className="min-w-0 flex-1">
                  <span className="block truncate text-sm font-medium">
                    {strip.name}
                  </span>
                  <span className="text-muted-foreground block truncate font-mono text-xs">
                    {strip.topic}
                  </span>
                </span>
                <Button
                  variant="ghost"
                  aria-label={`Remove ${strip.name}`}
                  disabled={saving}
                  onClick={() =>
                    onChange(strips.filter((s) => s.name !== strip.name))
                  }
                >
                  <Trash2 aria-hidden />
                </Button>
              </div>
              <div
                role="radiogroup"
                aria-label={`What ${strip.name} is made of`}
                className="flex flex-wrap gap-2"
              >
                {CHANNELS.map((choice) => (
                  <button
                    key={choice.id}
                    type="button"
                    role="radio"
                    aria-checked={current === choice.id}
                    title={choice.hint}
                    disabled={saving}
                    onClick={() => setChannels(strip, choice.id)}
                    className={cn(
                      "ring-border rounded-full px-3 py-1 text-xs ring-1 transition-colors",
                      "hover:bg-background focus-visible:ring-3 focus-visible:ring-ring/50 focus-visible:outline-none",
                      current === choice.id
                        ? "bg-background font-medium"
                        : "text-muted-foreground",
                    )}
                  >
                    {choice.label}
                  </button>
                ))}
              </div>
            </div>
          );
        })}

        <form onSubmit={add} className="flex flex-col gap-2 sm:flex-row">
          <Input
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder="living_room/ceiling"
            aria-label="Strip name"
            className="sm:w-52"
          />
          <Input
            value={topic}
            onChange={(e) => setTopic(e.target.value)}
            placeholder="wled/living_room"
            aria-label="Strip topic"
            className="font-mono sm:flex-1"
          />
          <Button type="submit" variant="outline" disabled={saving}>
            <Plus aria-hidden /> Add
          </Button>
        </form>

        {(problem || error) && (
          <p className="text-destructive text-sm">{problem ?? error}</p>
        )}
      </CardContent>
    </Card>
  );
}
