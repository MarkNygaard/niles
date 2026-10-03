import { useState } from "react";
import {
  Card,
  CardContent,
  CardDescription,
  CardFooter,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { CommitField } from "@/components/CommitField";
import { shouldOffer } from "@/components/PairPhoneCard";
import type { DeviceView, Me, MeUpdate } from "@/lib/api";

/** Niles reads the notes on every turn, so they are kept to a page. */
export const NOTES_LIMIT = 1500;

const MONTHS = [
  "January",
  "February",
  "March",
  "April",
  "May",
  "June",
  "July",
  "August",
  "September",
  "October",
  "November",
  "December",
];

const MONTH_ITEMS = MONTHS.map((label, i) => ({ label, value: String(i + 1) }));

export interface BirthdayParts {
  year: string;
  month: string;
  day: string;
}

/** "1990-10-03" or "10-03" as its parts, or blanks for none. */
export function birthdayParts(birthday: string | null): BirthdayParts {
  if (!birthday) return { year: "", month: "", day: "" };
  const parts = birthday.split("-");
  const [y, m, d] = parts.length === 3 ? parts : ["", ...parts];
  return { year: y, month: String(Number(m)), day: String(Number(d)) };
}

/**
 * "YYYY-MM-DD", or "MM-DD" without a year — `""` when all are cleared,
 * `null` while day or month is missing, or the day does not exist that
 * month (in that year, when there is one; 29 February otherwise counts).
 */
export function birthdayFrom(
  { year, month, day }: BirthdayParts,
  thisYear = new Date().getFullYear(),
): string | null {
  if (!year && !month && !day) return "";
  const y = year ? Number(year) : 2024;
  const m = Number(month);
  const d = Number(day);
  if (!Number.isInteger(m) || !Number.isInteger(d) || m < 1 || m > 12 || d < 1) return null;
  if (year && (!/^\d{4}$/.test(year) || y < 1900 || y > thisYear)) return null;
  if (d > new Date(y, m, 0).getDate()) return null;
  const md = `${String(m).padStart(2, "0")}-${String(d).padStart(2, "0")}`;
  return year ? `${year}-${md}` : md;
}

export interface MeCardProps {
  /** Undefined while it is being fetched. */
  me?: Me;
  device?: DeviceView;
  saving?: boolean;
  error?: string;
  onSave: (update: MeUpdate) => void;
  onPair: () => void;
  onUnpair: () => void;
}

/**
 * The signed-in person's own page.
 *
 * Theirs alone: nobody else in the house can open it, and Niles reads
 * the notes only when it is their voice it hears. So it can hold what a
 * household page should not — the name you would rather be called, the
 * tea you take — and Niles adds to it when told "remember that I…".
 */
export function MeCard({ me, device, saving, error, onSave, onPair, onUnpair }: MeCardProps) {
  if (!me) return null;

  return (
    <div className="flex flex-col gap-4">
      {me.speaker === null ? (
        <Card>
          <CardHeader>
            <CardTitle>Niles does not know your voice yet</CardTitle>
            <CardDescription>
              Notes, a birthday and how Niles addresses you belong to a voice.
              Say “I am ” and your name to a satellite a few times, then link
              {` ${me.email} `}to that voice under People.
            </CardDescription>
          </CardHeader>
        </Card>
      ) : (
        <>
          <NotesCard
            notes={me.notes ?? ""}
            name={me.display_name ?? me.speaker}
            saving={saving}
            onSave={(notes) => onSave({ notes })}
          />
          <Card>
            <CardHeader>
              <CardTitle>How Niles speaks to you</CardTitle>
              <CardDescription>
                Signed in as {me.email}, and known by the voice “{me.display_name}”.
              </CardDescription>
            </CardHeader>
            <CardContent className="flex flex-col gap-4">
              <div className="flex flex-col gap-3 sm:flex-row">
                {/* Beside the name, never instead of it: Niles still
                    knows who it is talking to. */}
                <CommitField
                  label="Address me as"
                  value={me.address_as ?? ""}
                  placeholder="Your name, or e.g. Sir"
                  allowEmpty
                  disabled={saving}
                  onCommit={(address_as) => onSave({ address_as })}
                />
                {/* Piper reads letters, not phonemes, so a name it
                    mispronounces is respelled until it sounds right. */}
                <CommitField
                  label="Say my name like"
                  value={me.spoken_as ?? ""}
                  placeholder={me.display_name ?? ""}
                  allowEmpty
                  disabled={saving}
                  onCommit={(spoken_as) => onSave({ spoken_as })}
                />
              </div>
              <BirthdayField
                birthday={me.birthday}
                disabled={saving}
                onCommit={(birthday) => onSave({ birthday })}
              />
              {error && <p className="text-destructive text-sm">{error}</p>}
            </CardContent>
          </Card>
        </>
      )}
      <PhoneCard device={device} phone={me.phone} saving={saving} onPair={onPair} onUnpair={onUnpair} />
    </div>
  );
}

/**
 * The notes, as a page you edit and save. Not saved per keystroke or on
 * blur like a name: a paragraph half rewritten is not something Niles
 * should start reading.
 */
function NotesCard({
  notes,
  name,
  saving,
  onSave,
}: {
  notes: string;
  name: string;
  saving?: boolean;
  onSave: (notes: string) => void;
}) {
  const [draft, setDraft] = useState(notes);
  const [seen, setSeen] = useState(notes);
  if (seen !== notes) {
    setSeen(notes);
    setDraft(notes);
  }
  const over = draft.trim().length > NOTES_LIMIT;
  const changed = draft !== notes;

  return (
    <Card>
      <CardHeader>
        <CardTitle>What Niles knows about you</CardTitle>
        <CardDescription>
          Read by Niles whenever it hears {name}, and by nobody else. Tell it
          “remember that I…” and it adds a line here; edit anything it got
          wrong.
        </CardDescription>
      </CardHeader>
      <CardContent className="flex flex-col gap-2">
        <Textarea
          id="me-notes"
          aria-label="Your notes"
          value={draft}
          disabled={saving}
          placeholder={"- Takes tea, not coffee\n- Works from home on Fridays"}
          className="min-h-40 font-mono text-sm"
          aria-invalid={over || undefined}
          onChange={(e) => setDraft(e.target.value)}
        />
        <p
          className={
            over ? "text-destructive text-xs tabular-nums" : "text-muted-foreground text-xs tabular-nums"
          }
        >
          {draft.trim().length} / {NOTES_LIMIT}
          {over && " — Niles reads this on every turn, so keep it to a page"}
        </p>
      </CardContent>
      <CardFooter className="justify-end gap-2">
        <Button variant="ghost" size="sm" disabled={!changed || saving} onClick={() => setDraft(notes)}>
          Discard
        </Button>
        <Button size="sm" disabled={!changed || over || saving} onClick={() => onSave(draft.trim())}>
          Save notes
        </Button>
      </CardFooter>
    </Card>
  );
}

/** Day and month, and the year if you want Niles to know your age. */
function BirthdayField({
  birthday,
  disabled,
  onCommit,
}: {
  birthday: string | null;
  disabled?: boolean;
  onCommit: (birthday: string) => void;
}) {
  const [parts, setParts] = useState(birthdayParts(birthday));
  const [seen, setSeen] = useState(birthday);
  if (seen !== birthday) {
    setSeen(birthday);
    setParts(birthdayParts(birthday));
  }

  const commit = (next: BirthdayParts) => {
    const value = birthdayFrom(next);
    if (value !== null && value !== (birthday ?? "")) onCommit(value);
  };
  const invalid =
    birthdayFrom(parts) === null && parts.month !== "" && parts.day !== "";
  const digits = (field: "day" | "year", max: number) => ({
    value: parts[field],
    disabled,
    inputMode: "numeric" as const,
    "aria-invalid": invalid || undefined,
    onChange: (e: React.ChangeEvent<HTMLInputElement>) =>
      setParts({ ...parts, [field]: e.target.value.replace(/\D/g, "").slice(0, max) }),
    onBlur: () => commit(parts),
    onKeyDown: (e: React.KeyboardEvent<HTMLInputElement>) => {
      if (e.key === "Enter") e.currentTarget.blur();
    },
  });

  return (
    <div className="flex flex-col gap-1">
      <span className="text-muted-foreground text-xs">Birthday</span>
      <div className="flex flex-wrap items-center gap-2">
        <Input
          id="me-birthday-day"
          aria-label="Birthday day"
          placeholder="Day"
          className="w-16"
          {...digits("day", 2)}
        />
        <Select
          items={MONTH_ITEMS}
          value={parts.month || null}
          disabled={disabled}
          onValueChange={(next: string | null) => {
            const updated = { ...parts, month: next ?? "" };
            setParts(updated);
            commit(updated);
          }}
        >
          <SelectTrigger aria-label="Birthday month" className="h-8 w-40">
            <SelectValue placeholder="Month" />
          </SelectTrigger>
          <SelectContent>
            <SelectGroup>
              {MONTH_ITEMS.map((m) => (
                <SelectItem key={m.value} value={m.value}>
                  {m.label}
                </SelectItem>
              ))}
            </SelectGroup>
          </SelectContent>
        </Select>
        <Input
          id="me-birthday-year"
          aria-label="Birthday year"
          placeholder="Year"
          className="w-20"
          {...digits("year", 4)}
        />
        {birthday && (
          <Button
            variant="ghost"
            size="sm"
            disabled={disabled}
            onClick={() => {
              setParts({ year: "", month: "", day: "" });
              onCommit("");
            }}
          >
            Clear
          </Button>
        )}
      </div>
      <span className="text-muted-foreground text-xs">
        {invalid
          ? "That is not a date — check the day and the year."
          : "On the day, Niles opens the morning with happy birthday. The year is optional; with it, Niles knows your age."}
      </span>
    </div>
  );
}

/** Which phone presence follows you by. */
function PhoneCard({
  device,
  phone,
  saving,
  onPair,
  onUnpair,
}: {
  device?: DeviceView;
  phone: string | null;
  saving?: boolean;
  onPair: () => void;
  onUnpair: () => void;
}) {
  const thisOne = device?.paired === true;
  return (
    <Card>
      <CardHeader>
        <CardTitle>My phone</CardTitle>
        <CardDescription>
          Niles knows you are home the moment your phone joins the Wi-Fi.
        </CardDescription>
      </CardHeader>
      <CardContent className="flex items-center justify-between gap-3">
        <span className="min-w-0 text-sm">
          {phone ? (
            <>
              {thisOne ? "This phone" : "A phone"} is paired
              <span className="text-muted-foreground block font-mono text-xs">{phone}</span>
            </>
          ) : shouldOffer(device) ? (
            <>Pair {device?.name ?? "this phone"} to be seen arriving.</>
          ) : (
            <span className="text-muted-foreground">
              None paired. Open this page on your phone, on the home Wi-Fi, to pair it.
            </span>
          )}
        </span>
        {phone ? (
          <Button variant="outline" size="sm" disabled={saving} onClick={onUnpair}>
            Unpair
          </Button>
        ) : (
          shouldOffer(device) && (
            <Button size="sm" disabled={saving} onClick={onPair}>
              This is my phone
            </Button>
          )
        )}
      </CardContent>
    </Card>
  );
}
