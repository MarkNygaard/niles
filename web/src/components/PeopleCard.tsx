import { useState } from "react";
import { Trash2, UserPlus } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { cn } from "@/lib/utils";

export interface Person {
  email: string;
  speaker?: string;
}

export interface PeopleCardProps {
  people: Person[];
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
export function PeopleCard({ people, saving, error, onChange }: PeopleCardProps) {
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
                {person.speaker && (
                  <div className="text-muted-foreground text-xs">
                    same person as “{person.speaker}” by voice
                  </div>
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
