import { useEffect, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { CalendarDays } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
    Card,
    CardContent,
    CardDescription,
    CardHeader,
    CardTitle,
} from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { api } from "@/lib/api";
import type { CalendarAccess, CalendarConnection } from "@/lib/api";
import { cn } from "@/lib/utils";

const KEY = ["calendars"];

/** The link that starts Microsoft's sign-in: followed, not fetched,
    because it ends at Microsoft. */
export function connectUrl(
    name: string,
    access: CalendarAccess,
    replace?: string,
): string {
    const query = new URLSearchParams({ name: name.trim(), access });
    if (replace) query.set("replace", replace);
    return `/calendars/microsoft/start?${query}`;
}

/** What the sign-in came back saying, from the address it returned to. */
export function outcomeOf(
    search: string,
): { ok: boolean; text: string } | null {
    const query = new URLSearchParams(search);
    const error = query.get("calendar_error");
    if (error) return { ok: false, text: error };
    if (query.get("calendar") === "connected") {
        const name = query.get("name");
        return {
            ok: true,
            text: name ? `${name} is connected.` : "Connected.",
        };
    }
    return null;
}

const ACCESS: { value: CalendarAccess; label: string; hint: string }[] = [
    {
        value: "read",
        label: "Read only",
        hint: "Niles can tell you what's on it.",
    },
    {
        value: "write",
        label: "Read and write",
        hint: "Niles can also add, move and delete events.",
    },
];

/**
 * The calendars Niles reads for you, and only for you: by your voice,
 * or here, signed in. Each one is a Microsoft sign-in of its own, and
 * what it may do is what Microsoft was asked to allow.
 */
export function CalendarsCard() {
    const queryClient = useQueryClient();
    const calendars = useQuery({
        queryKey: KEY,
        queryFn: api.calendars,
        retry: false,
    });
    const remove = useMutation({
        mutationFn: api.removeCalendar,
        onSettled: () => queryClient.invalidateQueries({ queryKey: KEY }),
    });
    const [name, setName] = useState("");
    const [access, setAccess] = useState<CalendarAccess>("read");
    const [outcome, setOutcome] = useState(() =>
        outcomeOf(window.location.search),
    );

    // Said once, then taken off the address so a refresh does not say it
    // again.
    useEffect(() => {
        if (!outcome) return;
        const clean = window.location.pathname + window.location.hash;
        window.history.replaceState(null, "", clean);
    }, [outcome]);

    const go = (url: string) => {
        setOutcome(null);
        window.location.assign(url);
    };

    return (
        <Card>
            <CardHeader>
                <CardTitle className="flex items-center gap-2">
                    <CalendarDays aria-hidden className="size-4" /> Calendars
                </CardTitle>
                <CardDescription>
                    Read by Niles only when it hears you, or when you ask here.
                    Nobody else can ask for yours.
                </CardDescription>
            </CardHeader>
            <CardContent className="flex flex-col gap-4">
                {outcome && (
                    <p
                        role="status"
                        className={cn(
                            "text-sm",
                            outcome.ok ? "text-foreground" : "text-destructive",
                        )}
                    >
                        {outcome.text}
                    </p>
                )}
                {calendars.isError && (
                    <p className="text-muted-foreground text-sm">
                        {calendars.error.message}
                    </p>
                )}
                {calendars.data &&
                    !calendars.data.available &&
                    calendars.data.reason && (
                        <p className="text-muted-foreground text-sm">
                            {calendars.data.reason}
                        </p>
                    )}

                {calendars.data?.connections.map((c) => (
                    <ConnectionRow
                        key={c.id}
                        connection={c}
                        busy={remove.isPending}
                        onChangeAccess={() =>
                            go(
                                connectUrl(
                                    c.name,
                                    c.access === "read" ? "write" : "read",
                                    c.id,
                                ),
                            )
                        }
                        onRemove={() => remove.mutate(c.id)}
                    />
                ))}

                {calendars.data?.available && (
                    <form
                        className="flex flex-col gap-3 border-t pt-4"
                        onSubmit={(e) => {
                            e.preventDefault();
                            if (name.trim()) go(connectUrl(name, access));
                        }}
                    >
                        <span className="text-sm font-medium">
                            Connect a calendar
                        </span>
                        <Input
                            aria-label="Calendar name"
                            placeholder="Private, Work…"
                            maxLength={40}
                            value={name}
                            onChange={(e) => setName(e.target.value)}
                        />
                        <div
                            role="radiogroup"
                            aria-label="What Niles may do"
                            className="flex flex-col gap-2 sm:flex-row"
                        >
                            {ACCESS.map((option) => (
                                <button
                                    key={option.value}
                                    type="button"
                                    role="radio"
                                    aria-checked={access === option.value}
                                    onClick={() => setAccess(option.value)}
                                    className={cn(
                                        "flex-1 rounded-lg border px-3 py-2 text-left transition-colors",
                                        "focus-visible:ring-3 focus-visible:ring-ring/50 focus-visible:outline-none",
                                        access === option.value
                                            ? "border-foreground bg-muted"
                                            : "hover:bg-muted/50",
                                    )}
                                >
                                    <span className="block text-sm font-medium">
                                        {option.label}
                                    </span>
                                    <span className="text-muted-foreground block text-xs">
                                        {option.hint}
                                    </span>
                                </button>
                            ))}
                        </div>
                        <Button
                            type="submit"
                            className="self-start"
                            disabled={!name.trim()}
                        >
                            Connect with Microsoft
                        </Button>
                        <p className="text-muted-foreground text-xs">
                            A work account may need your IT department's
                            approval before it can be connected.
                        </p>
                    </form>
                )}
            </CardContent>
        </Card>
    );
}

function ConnectionRow({
    connection,
    busy,
    onChangeAccess,
    onRemove,
}: {
    connection: CalendarConnection;
    busy: boolean;
    onChangeAccess: () => void;
    onRemove: () => void;
}) {
    const [confirming, setConfirming] = useState(false);
    return (
        <div className="flex flex-col gap-2 rounded-lg border px-3 py-2 sm:flex-row sm:items-center">
            <div className="min-w-0 flex-1">
                <div className="flex items-center gap-2">
                    <span className="truncate text-sm font-medium">
                        {connection.name}
                    </span>
                    <Badge variant="outline">
                        {connection.access === "write"
                            ? "Read and write"
                            : "Read only"}
                    </Badge>
                </div>
                <div className="text-muted-foreground truncate text-xs">
                    {connection.account}
                </div>
            </div>
            <div className="flex gap-2">
                <Button
                    variant="outline"
                    size="sm"
                    disabled={busy}
                    onClick={onChangeAccess}
                >
                    {connection.access === "write"
                        ? "Make read only"
                        : "Allow writing"}
                </Button>
                {confirming ? (
                    <Button
                        variant="destructive"
                        size="sm"
                        disabled={busy}
                        onClick={onRemove}
                    >
                        Disconnect {connection.name}
                    </Button>
                ) : (
                    <Button
                        variant="ghost"
                        size="sm"
                        disabled={busy}
                        onClick={() => setConfirming(true)}
                    >
                        Disconnect
                    </Button>
                )}
            </div>
        </div>
    );
}
