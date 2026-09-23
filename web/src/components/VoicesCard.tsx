import { useState } from "react";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Switch } from "@/components/ui/switch";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Trash2 } from "lucide-react";
import type { Voice } from "@/lib/api";

/**
 * What to say about a voice, in one line.
 *
 * `last_seen_at` is the number that matters and the one nobody would
 * think to ask for: a voice enrolled and never matched since is
 * exactly what a failing enrolment looks like, and it looks identical
 * to a working one everywhere else.
 */
export function voiceSummary(voice: Voice): string {
  const clips =
    voice.clip_count === 1 ? "1 clip" : `${voice.clip_count} clips`;
  const thin = voice.clip_count < 3 ? " — thin, say the name again" : "";
  const heard = voice.last_seen_at
    ? `heard ${new Date(voice.last_seen_at).toLocaleDateString()}`
    : "never recognised since";
  return `${clips}${thin} · ${heard}`;
}

export interface VoicesCardProps {
  /** Whether a voice Niles does not know is answered at all. */
  knownVoicesOnly: boolean;
  /** Whether recognition is running. The lock needs it to mean anything. */
  recognitionOn: boolean;
  /** Who Niles has been taught. Undefined while it is being fetched. */
  voices?: Voice[];
  saving?: boolean;
  onChange: (knownVoicesOnly: boolean) => void;
  /** Forget one entirely — the way to start a voice over. */
  onForget?: (speaker: string) => void;
  /** Correct the name, which came from a transcript and is a guess. */
  onRename?: (speaker: string, displayName: string) => void;
  /** Respell it so Piper says it right. Empty clears the respelling. */
  onSpokenAs?: (speaker: string, spokenAs: string) => void;
}

/**
 * Whether Niles answers a voice it does not know.
 *
 * One switch rather than two, because the question a household asks is
 * "does Niles trust this voice", and the answer decides both whether
 * it acts and whether it will learn a new name. Splitting them would
 * mean explaining, on a settings page, why a stranger who cannot turn
 * a light on can still tell the house who they are.
 *
 * That pairing is also the point. A satellite listens to a room, and a
 * television in that room says sentences — one of them said "Russia
 * still has no knowledge of holding your love, correct?" and Niles
 * answered it. With the lock on, a voice nobody has introduced cannot
 * act *or* introduce itself, which is what stops a programme becoming
 * a resident.
 */
export function VoicesCard({
  knownVoicesOnly,
  recognitionOn,
  voices,
  saving,
  onChange,
  onForget,
  onRename,
  onSpokenAs,
}: VoicesCardProps) {
  return (
    <Card>
      <CardHeader>
        <CardTitle>Voices</CardTitle>
        <CardDescription>
          Niles learns a voice when someone says “I am ” and their name. A
          satellite listens to a whole room, so this decides whether it
          answers one it has never heard before.
        </CardDescription>
      </CardHeader>
      <CardContent className="flex flex-col gap-3">
        {/* The list first. Who Niles knows is the question somebody
            opens this card with; the lock is what they do about it. */}
        {voices !== undefined && (
          <div className="flex flex-col gap-2 border-b pb-3">
            {voices.length === 0 ? (
              <p className="text-muted-foreground text-sm">
                Nobody yet. Say “my name is ” and your name to a satellite,
                three or four times, from where you usually speak.
              </p>
            ) : (
              voices.map((voice) => (
                <div
                  key={voice.speaker}
                  className="flex items-center justify-between gap-3"
                >
                  <div className="min-w-0 flex-1">
                    {onRename ? (
                      <NameField
                        value={voice.display_name}
                        label={`${voice.speaker} name`}
                        disabled={saving}
                        onCommit={(name) => onRename(voice.speaker, name)}
                      />
                    ) : (
                      <div className="truncate text-sm font-medium">
                        {voice.display_name}
                      </div>
                    )}
                    <div className="text-muted-foreground text-xs">
                      {voiceSummary(voice)}
                    </div>
                    {/* Piper reads letters, not phonemes, so a name it
                        mispronounces is respelled until it sounds
                        right. Separate from the name above because
                        "Mayse" is how you say it and not how it is
                        written. */}
                    {onSpokenAs && (
                      <NameField
                        value={voice.spoken_as ?? ""}
                        label={`${voice.speaker} pronunciation`}
                        placeholder="Say it like…"
                        allowEmpty
                        disabled={saving}
                        onCommit={(said) => onSpokenAs(voice.speaker, said)}
                      />
                    )}
                  </div>
                  {onForget && (
                    <Button
                      variant="ghost"
                      size="icon-sm"
                      aria-label={`Forget ${voice.display_name}`}
                      disabled={saving}
                      onClick={() => onForget(voice.speaker)}
                    >
                      <Trash2 aria-hidden />
                    </Button>
                  )}
                </div>
              ))
            )}
          </div>
        )}

        <label className="flex items-center justify-between gap-4">
          <span className="min-w-0">
            <span className="block text-sm font-medium">
              Only answer voices Niles knows
            </span>
            <span className="text-muted-foreground block text-xs">
              An unfamiliar voice is told so, and nothing happens — including
              introducing itself, which is what keeps the television from
              becoming a resident.
            </span>
          </span>
          <Switch
            checked={knownVoicesOnly}
            disabled={saving}
            // Wrapped rather than passed straight through: Base UI
            // calls this with the event details as a second argument,
            // and the prop above promises one.
            onCheckedChange={(next) => onChange(next)}
          />
        </label>

        {/* The chicken-and-egg, said before it is hit rather than after. */}
        {knownVoicesOnly && (
          <p className="text-muted-foreground border-t pt-3 text-xs">
            To add somebody, switch this off, have them say “I am ” and their
            name to a satellite a few times, then switch it back on.
          </p>
        )}

        {/* A switch that governs something not running would otherwise
            look like it had been obeyed. */}
        {!recognitionOn && (
          <p className="text-muted-foreground border-t pt-3 text-xs">
            Niles is not set up to recognise voices yet, so this has no effect
            until it is. Every voice is answered in the meantime.
          </p>
        )}
      </CardContent>
    </Card>
  );
}

/**
 * The name, correctable.
 *
 * Whisper spelled one Danish name four ways in four attempts, and the
 * first spelling is what the slug is stuck with. What anybody reads
 * does not have to be. Commits on blur rather than per keystroke: each
 * one is a write and a matcher rebuild.
 */
function NameField({
  value,
  label,
  placeholder,
  allowEmpty,
  disabled,
  onCommit,
}: {
  value: string;
  label: string;
  placeholder?: string;
  /** Whether clearing it is a value in its own right. */
  allowEmpty?: boolean;
  disabled?: boolean;
  onCommit: (value: string) => void;
}) {
  const [draft, setDraft] = useState(value);
  const [seen, setSeen] = useState(value);
  if (seen !== value) {
    setSeen(value);
    setDraft(value);
  }

  return (
    <Input
      value={draft}
      aria-label={label}
      placeholder={placeholder}
      disabled={disabled}
      spellCheck={false}
      className="h-7 border-transparent px-1 text-sm font-medium hover:border-input"
      onChange={(e) => setDraft(e.target.value)}
      onBlur={() => {
        const next = draft.trim();
        if (next !== value && (allowEmpty || next)) onCommit(next);
        else setDraft(value);
      }}
      onKeyDown={(e) => {
        if (e.key === "Enter") e.currentTarget.blur();
        if (e.key === "Escape") setDraft(value);
      }}
    />
  );
}
