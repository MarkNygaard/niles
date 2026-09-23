import { useState } from "react";
import { Trash2, UserPlus } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";

/** The sentinel for "paired with nobody", which is not a voice. */
const NO_VOICE = " none";
import { cn } from "@/lib/utils";

export interface Person {
  email: string;
  speaker?: string;
}

/**
 * The voices a person can be paired with.
 *
 * Whatever is already configured is kept even when no voice by that
 * name is enrolled — a pairing that points at a deleted voice is worth
 * showing rather than silently dropping, since the fix is to repair it
 * and you cannot repair what the page will not display.
 */
export function speakerChoices(
  enrolled: { speaker: string; display_name: string }[] | undefined,
  current: string | undefined,
): { speaker: string; display_name: string }[] {
  const list = enrolled ?? [];
  if (!current || list.some((v) => v.speaker === current)) return list;
  // A pairing whose voice has been deleted has no name to show, so it
  // shows its slug — which is still better than vanishing, because the
  // fix is to repair it and you cannot repair what is not displayed.
  return [...list, { speaker: current, display_name: current }];
}

export interface PeopleCardProps {
  people: Person[];
  /**
   * The enrolled voices, for pairing. Undefined while loading.
   *
   * Both halves are needed: the slug is what gets saved, and the name
   * is what anybody can recognise. Offering the slug alone showed
   * "maisel" — which is what Whisper heard, not what she is called,
   * and changing the name left the list still saying it.
   */
  voices?: { speaker: string; display_name: string }[];
  saving?: boolean;
  error?: string;
  onChange: (people: Person[]) => void;
}

/**
 * Who can sign in.
 *
 * GitHub is only the front door — accounts there are free, so this list
 * is the boundary. Which is why it is a list of addresses rather than
 * an invitation flow: adding somebody is an edit, so there is no
 * message to send and nothing to expire.
 */
export function PeopleCard({
  people,
  voices,
  saving,
  error,
  onChange,
}: PeopleCardProps) {
  const [draft, setDraft] = useState("");
  const [problem, setProblem] = useState<string | null>(null);

  function add(event: React.FormEvent) {
    event.preventDefault();
    const email = draft.trim();
    if (!email) return;
    // The server checks this too. Saying it here means the answer
    // arrives while the address is still on screen to fix.
    if (!looksLikeEmail(email)) {
      setProblem(`${email} is not an email address.`);
      return;
    }
    if (people.some((person) => person.email.toLowerCase() === email.toLowerCase())) {
      setProblem(`${email} is already on the list.`);
      return;
    }
    setProblem(null);
    setDraft("");
    onChange([...people, { email }]);
  }

  const last = people.length === 1;

  return (
    <div className="flex flex-col gap-4">
      {people.length === 0 ? (
        <p className="bg-muted/60 rounded-md px-3 py-2 text-sm">
          Signing in is off while nobody is listed, so Niles is open to anyone
          who can reach it. Add yourself to switch it on.
        </p>
      ) : (
        <ul className="divide-border divide-y">
          {people.map((person) => (
            <li
              key={person.email}
              className="flex items-center justify-between gap-3 py-2"
            >
              <div className="min-w-0">
                <div className="truncate text-sm">{person.email}</div>
                {voices !== undefined && (
                  <Select
                    value={person.speaker ?? NO_VOICE}
                    disabled={saving}
                    onValueChange={(next: string | null) =>
                      onChange(
                        people.map((other) =>
                          other.email === person.email
                            ? {
                                ...other,
                                // Omitted rather than blank: the config
                                // refuses an empty speaker and says to
                                // leave the key out instead.
                                speaker:
                                  !next || next === NO_VOICE
                                    ? undefined
                                    : next,
                              }
                            : other,
                        ),
                      )
                    }
                  >
                    <SelectTrigger
                      aria-label={`${person.email} voice`}
                      className="mt-1 h-7 w-full text-xs"
                    >
                      {/* Base UI renders the raw value unless told
                          otherwise, and the value is the slug — so the
                          list said "Majse" and the box said "maisel"
                          the moment you picked it. */}
                      <SelectValue placeholder="No voice">
                        {(value: string) =>
                          value === NO_VOICE
                            ? "No voice"
                            : (voices?.find((v) => v.speaker === value)
                                ?.display_name ?? value)
                        }
                      </SelectValue>
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value={NO_VOICE}>No voice</SelectItem>
                      {speakerChoices(voices, person.speaker).map((v) => (
                        <SelectItem key={v.speaker} value={v.speaker}>
                          {v.display_name}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                )}
              </div>
              <Button
                variant="ghost"
                size="icon-sm"
                disabled={saving || last}
                aria-label={
                  last
                    ? `Remove ${person.email} — not while they are the only one`
                    : `Remove ${person.email}`
                }
                title={
                  last
                    ? "Add somebody else first: with nobody listed, nobody could sign in to put them back."
                    : undefined
                }
                onClick={() =>
                  onChange(people.filter((other) => other.email !== person.email))
                }
                className={cn(last && "cursor-not-allowed")}
              >
                <Trash2 />
              </Button>
            </li>
          ))}
        </ul>
      )}

      {/* `noValidate` with `type="email"` still: the type is what gets
          the right keyboard on a phone, but the browser's own check
          would block the submit and answer with "please include an @",
          which is not the mistake people actually make here. Ours says
          it is not a username. */}
      <form onSubmit={add} noValidate className="flex flex-col gap-1">
        <div className="flex flex-wrap items-center gap-2">
          <Input
            value={draft}
            type="email"
            inputMode="email"
            autoComplete="off"
            spellCheck={false}
            disabled={saving}
            aria-label="Email address to allow"
            placeholder="them@example.com"
            className="w-64"
            onChange={(e) => {
              setDraft(e.target.value);
              setProblem(null);
            }}
          />
          <Button type="submit" variant="outline" disabled={saving || !draft.trim()}>
            <UserPlus /> Add
          </Button>
        </div>
        <p className="text-muted-foreground pl-0.5 text-xs">
          The address GitHub has <em>verified</em> for them — GitHub → Settings →
          Emails. Not their username, and not an unverified address.
        </p>
      </form>

      {(problem || error) && (
        <p className="text-destructive text-sm">{problem ?? error}</p>
      )}
    </div>
  );
}

/**
 * Enough to catch the common mistake, which is pasting a GitHub
 * username. Anything subtler is the server's to refuse, and it does.
 */
function looksLikeEmail(value: string): boolean {
  const [local, domain, ...rest] = value.split("@");
  return (
    rest.length === 0 &&
    !!local &&
    !!domain &&
    domain.includes(".") &&
    !domain.startsWith(".") &&
    !domain.endsWith(".")
  );
}
