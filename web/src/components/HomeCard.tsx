import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Check, Loader2, MapPin, Search } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import {
  Combobox,
  ComboboxContent,
  ComboboxEmpty,
  ComboboxInput,
  ComboboxItem,
  ComboboxItemIndicator,
  ComboboxList,
} from "@/components/ui/combobox";
import { Input } from "@/components/ui/input";
import { cn } from "@/lib/utils";
import { api } from "@/lib/api";
import type { Place } from "@/lib/api";

export interface HomeValues {
  name?: string;
  latitude?: number;
  longitude?: number;
  timezone?: string;
  units?: string;
}

export interface HomeCardProps {
  values: HomeValues;
  timezones: string[];
  saving?: boolean;
  error?: string;
  onChange: (entries: { path: string; value: unknown }[]) => void;
  /** Drop an override, which is the only way back to "not set".
      TOML has no null, so a patch cannot express it. */
  onClear: (path: string) => void;
}

const UNITS = [
  { id: "metric", label: "Metric", hint: "°C, km" },
  { id: "imperial", label: "Imperial", hint: "°F, miles" },
] as const;

/** Null Island, which is what an unanswered location looks like. */
function isUnset(values: HomeValues): boolean {
  return !values.latitude && !values.longitude;
}

/**
 * Where the house is, and what clock it keeps.
 *
 * Three of these are the settings a first start gets wrong in a way
 * nothing complains about. A timezone left at UTC runs the whole
 * lighting curve an hour or two out and looks deliberate; coordinates
 * left at nothing put the weather in the Gulf of Guinea. Neither has a
 * value that could be guessed, so both are reported as gaps instead —
 * and this is the page that closes them.
 *
 * The search is there because nobody knows their own latitude. It
 * finds places rather than street addresses, which is the right
 * precision for a regional forecast, and it comes back with the
 * timezone as well — asking two services the same question would be
 * two chances for them to disagree.
 */
export function HomeCard({
  values,
  timezones,
  saving,
  error,
  onChange,
  onClear,
}: HomeCardProps) {
  const [query, setQuery] = useState("");
  const [submitted, setSubmitted] = useState("");
  const [picked, setPicked] = useState<string | null>(null);

  const places = useQuery({
    queryKey: ["places", submitted],
    queryFn: () => api.places(submitted),
    enabled: submitted.length > 0,
  });

  function choose(place: Place) {
    setPicked(place.label);
    // One edit, not four. They describe a single place, and applying
    // them one at a time would leave the config briefly claiming a
    // latitude in Denmark and a timezone in UTC.
    const entries = [
      { path: "home.latitude", value: place.latitude },
      { path: "home.longitude", value: place.longitude },
      { path: "home.timezone", value: place.timezone },
    ];
    if (place.country_code) {
      entries.push({ path: "home.country", value: place.country_code });
    }
    onChange(entries);
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle>Where the house is</CardTitle>
        <CardDescription>
          The timezone runs the lighting curve, so a wrong one shifts the
          whole day. The coordinates are only read by the weather. Search for
          the nearest town and both are filled in — a house number would
          change the fourth decimal and nothing else.
        </CardDescription>
      </CardHeader>
      <CardContent className="flex flex-col gap-5">
        <form
          className="flex flex-col gap-2 sm:flex-row"
          onSubmit={(event) => {
            event.preventDefault();
            setPicked(null);
            setSubmitted(query.trim());
          }}
        >
          <Input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Aarhus"
            aria-label="Town or city"
            className="sm:flex-1"
          />
          <Button type="submit" variant="outline" disabled={saving}>
            {places.isFetching ? (
              <Loader2 aria-hidden className="animate-spin" />
            ) : (
              <Search aria-hidden />
            )}
            Find it
          </Button>
        </form>

        {places.isError && (
          <p className="text-destructive text-sm">
            The place index could not be reached. The numbers below can be
            typed in by hand.
          </p>
        )}

        {places.data && places.data.length === 0 && (
          <p className="text-muted-foreground text-sm">
            Nothing matched “{submitted}”. Try the nearest larger town, or
            type the numbers in below.
          </p>
        )}

        {places.data && places.data.length > 0 && (
          <ul className="flex flex-col gap-1">
            {places.data.map((place) => (
              <li key={`${place.latitude},${place.longitude}`}>
                <button
                  type="button"
                  disabled={saving}
                  onClick={() => choose(place)}
                  className={cn(
                    "hover:bg-muted/60 focus-visible:ring-3 focus-visible:ring-ring/50 flex w-full items-center gap-3 rounded-lg px-3 py-2 text-left focus-visible:outline-none",
                    picked === place.label && "bg-muted/60",
                  )}
                >
                  {picked === place.label ? (
                    <Check aria-hidden className="size-4 shrink-0" />
                  ) : (
                    <MapPin
                      aria-hidden
                      className="text-muted-foreground size-4 shrink-0"
                    />
                  )}
                  <span className="min-w-0 flex-1">
                    <span className="block truncate text-sm">
                      {place.label}
                    </span>
                    <span className="text-muted-foreground block truncate font-mono text-xs">
                      {place.latitude.toFixed(4)}, {place.longitude.toFixed(4)}{" "}
                      · {place.timezone}
                    </span>
                  </span>
                </button>
              </li>
            ))}
          </ul>
        )}

        <div className="divide-border divide-y">
          <Field
            label="Name"
            hint="What Niles calls the place out loud."
          >
            <TextValue
              value={values.name ?? ""}
              placeholder="Home"
              label="Home name"
              disabled={saving}
              onCommit={(next) =>
                onChange([{ path: "home.name", value: next }])
              }
            />
          </Field>

          <Field
            label="Timezone"
            hint="Every time in the curve is read in this zone."
          >
            <Combobox
              items={timezones}
              value={values.timezone ?? "UTC"}
              disabled={saving}
              onValueChange={(next: string | null) => {
                if (next) onChange([{ path: "home.timezone", value: next }]);
              }}
            >
              <ComboboxInput aria-label="Timezone" className="w-full sm:w-72" />
              <ComboboxContent>
                <ComboboxEmpty>No zone matches that.</ComboboxEmpty>
                <ComboboxList>
                  {(zone: string) => (
                    <ComboboxItem key={zone} value={zone}>
                      <ComboboxItemIndicator>
                        <Check />
                      </ComboboxItemIndicator>
                      <span className="flex-1">{zone}</span>
                    </ComboboxItem>
                  )}
                </ComboboxList>
              </ComboboxContent>
            </Combobox>
          </Field>

          <Field
            label="Coordinates"
            hint={
              isUnset(values)
                ? "Unset, so weather answers are for the Gulf of Guinea."
                : "Decimal degrees. The search above fills these in."
            }
          >
            <div className="flex flex-wrap gap-2">
              <TextValue
                value={values.latitude?.toString() ?? ""}
                placeholder="56.1572"
                label="Latitude"
                numeric
                disabled={saving}
                onCommit={(next) =>
                  onChange([{ path: "home.latitude", value: Number(next) }])
                }
              />
              <TextValue
                value={values.longitude?.toString() ?? ""}
                placeholder="10.2107"
                label="Longitude"
                numeric
                disabled={saving}
                onCommit={(next) =>
                  onChange([{ path: "home.longitude", value: Number(next) }])
                }
              />
            </div>
          </Field>

          <Field
            label="Units"
            hint="What Niles says out loud. Left alone it follows the country."
          >
            <div role="radiogroup" aria-label="Units" className="flex gap-2">
              {UNITS.map((unit) => (
                <button
                  key={unit.id}
                  type="button"
                  role="radio"
                  aria-checked={values.units === unit.id}
                  title={unit.hint}
                  disabled={saving}
                  onClick={() =>
                    // Picking the one already set clears it, which is
                    // the way back to following the country — and less
                    // surprising than a third button called "neither".
                    values.units === unit.id
                      ? onClear("home.units")
                      : onChange([{ path: "home.units", value: unit.id }])
                  }
                  className={cn(
                    "ring-border rounded-full px-3 py-1 text-xs ring-1 transition-colors",
                    "hover:bg-background focus-visible:ring-3 focus-visible:ring-ring/50 focus-visible:outline-none",
                    values.units === unit.id
                      ? "bg-background font-medium"
                      : "text-muted-foreground",
                  )}
                >
                  {unit.label}
                </button>
              ))}
            </div>
          </Field>
        </div>

        {error && <p className="text-destructive text-sm">{error}</p>}
      </CardContent>
    </Card>
  );
}

function Field({
  label,
  hint,
  children,
}: {
  label: string;
  hint: string;
  children: React.ReactNode;
}) {
  return (
    <div className="grid gap-x-8 gap-y-3 py-4 sm:grid-cols-[minmax(190px,240px)_minmax(0,1fr)]">
      <div className="flex flex-col gap-1">
        <span className="text-sm font-medium">{label}</span>
        <p className="text-muted-foreground text-xs">{hint}</p>
      </div>
      <div className="min-w-0">{children}</div>
    </div>
  );
}

/**
 * A value that writes when you leave it, not on every keystroke.
 *
 * Same reason the curve's sliders commit on release: each write is a
 * real config revision, and half a latitude is not a place.
 */
function TextValue({
  value,
  placeholder,
  label,
  numeric,
  disabled,
  onCommit,
}: {
  value: string;
  placeholder: string;
  label: string;
  numeric?: boolean;
  disabled?: boolean;
  onCommit: (value: string) => void;
}) {
  const [draft, setDraft] = useState(value);
  const [seen, setSeen] = useState(value);
  if (seen !== value) {
    setSeen(value);
    setDraft(value);
  }

  const usable = draft.trim() !== "" && (!numeric || !Number.isNaN(Number(draft)));

  return (
    <Input
      value={draft}
      aria-label={label}
      placeholder={placeholder}
      disabled={disabled}
      inputMode={numeric ? "decimal" : "text"}
      aria-invalid={draft.trim() !== "" && !usable}
      className={cn("w-full", numeric ? "w-32 font-mono" : "sm:w-72")}
      onChange={(e) => setDraft(e.target.value)}
      onBlur={() => {
        if (draft === value || !usable) {
          setDraft(value);
          return;
        }
        onCommit(draft);
      }}
      onKeyDown={(e) => {
        if (e.key === "Enter") e.currentTarget.blur();
        if (e.key === "Escape") setDraft(value);
      }}
    />
  );
}
