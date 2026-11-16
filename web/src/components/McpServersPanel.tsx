import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
    Dialog,
    DialogBody,
    DialogContent,
    DialogDescription,
    DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Skeleton } from "@/components/ui/skeleton";
import { api } from "@/lib/api";
import type { PersonalServer, PersonalToolMode, ServerSignIn } from "@/lib/api";

export const MY_SERVERS = ["me-tool-servers"];

/** What a server's state reads as. */
export const STATUS: Record<PersonalServer["status"], string> = {
    connecting: "Connecting",
    working: "Connected",
    needs_key: "Needs its key",
    needs_sign_in: "Sign in needed",
    failed: "Not working",
};

const MODES: { mode: PersonalToolMode; label: string }[] = [
    { mode: "on", label: "On" },
    { mode: "ask", label: "Ask" },
    { mode: "off", label: "Off" },
];

/** What a browser sign-in's return left in the address's query, if
    anything: `#/me/profile/mcp?signed_in=3`. */
export function signInReturn(
    params: URLSearchParams,
): "done" | "failed" | null {
    if (params.get("sign_in") === "failed") return "failed";
    if (params.get("signed_in")) return "done";
    return null;
}

/**
 * My profile → MCP servers: the person's own remote MCP servers. Their
 * tools are theirs alone, and only in the written chat. A key is
 * write-only; a server that signs in on its own page is signed in from
 * here, never through the chat.
 */
export function McpServersPanel({
    returned,
}: {
    returned?: "done" | "failed" | null;
}) {
    const queryClient = useQueryClient();
    const list = useQuery({
        queryKey: MY_SERVERS,
        queryFn: api.myServers,
        retry: false,
    });
    const settle = () =>
        queryClient.invalidateQueries({ queryKey: MY_SERVERS });
    const [adding, setAdding] = useState(false);
    const [editing, setEditing] = useState<PersonalServer | null>(null);
    const [removing, setRemoving] = useState<PersonalServer | null>(null);

    const setMode = useMutation({
        mutationFn: (v: { id: number; tool: string; mode: PersonalToolMode }) =>
            api.setMyServerMode(v.id, v.tool, v.mode),
        onSettled: settle,
    });
    const reconnect = useMutation({
        mutationFn: api.reconnectMyServer,
        onSettled: settle,
    });
    const remove = useMutation({
        mutationFn: api.removeMyServer,
        onSuccess: () => setRemoving(null),
        onSettled: settle,
    });
    const signIn = useMutation({
        mutationFn: api.signInMyServer,
        onSuccess: ({ url }) => window.location.assign(url),
    });

    if (list.isPending) return <Skeleton className="h-10 w-full" />;
    if (list.isError) {
        return <p className="text-destructive text-sm">{list.error.message}</p>;
    }

    const servers = list.data;
    return (
        <>
            <div className="flex flex-col gap-4">
                {returned === "done" && (
                    <p className="text-sm">Signed in. Niles is connecting.</p>
                )}
                {returned === "failed" && (
                    <p className="text-destructive text-sm">
                        The sign-in didn't finish. Try again.
                    </p>
                )}
                {signIn.error && (
                    <p className="text-destructive text-sm">
                        {signIn.error.message}
                    </p>
                )}
                {servers.length === 0 ? (
                    <p className="text-muted-foreground text-sm">
                        No servers of your own yet. Add one by its address, with
                        a key or by signing in on its page.
                    </p>
                ) : (
                    <ul
                        aria-label="Your MCP servers"
                        className="flex flex-col gap-5"
                    >
                        {servers.map((server) => (
                            <li key={server.id} className="flex flex-col gap-2">
                                <div className="flex flex-wrap items-center gap-2">
                                    <span className="font-medium">
                                        {server.name}
                                    </span>
                                    <Badge
                                        variant={
                                            server.status === "working"
                                                ? "secondary"
                                                : "outline"
                                        }
                                    >
                                        {STATUS[server.status]}
                                    </Badge>
                                </div>
                                <p className="text-muted-foreground text-sm break-all">
                                    {server.url}
                                </p>
                                {server.message &&
                                    server.status !== "working" && (
                                        <p className="text-muted-foreground text-xs break-all">
                                            {server.message}
                                        </p>
                                    )}
                                {server.tools.length > 0 && (
                                    <ul
                                        aria-label={`Tools of ${server.name}`}
                                        className="flex flex-col gap-1.5"
                                    >
                                        {server.tools.map((tool) => (
                                            <li
                                                key={tool.name}
                                                className="flex flex-wrap items-center gap-2"
                                            >
                                                <span className="min-w-0 flex-1 text-sm break-all">
                                                    {tool.title ?? tool.name}
                                                    {tool.read_only && (
                                                        <span className="text-muted-foreground">
                                                            {" "}
                                                            · reads only
                                                        </span>
                                                    )}
                                                </span>
                                                <div
                                                    role="group"
                                                    aria-label={`${tool.name} setting`}
                                                    className="flex gap-1"
                                                >
                                                    {MODES.map(
                                                        ({ mode, label }) => (
                                                            <Button
                                                                key={mode}
                                                                size="sm"
                                                                variant={
                                                                    tool.mode ===
                                                                    mode
                                                                        ? "secondary"
                                                                        : "ghost"
                                                                }
                                                                aria-pressed={
                                                                    tool.mode ===
                                                                    mode
                                                                }
                                                                disabled={
                                                                    setMode.isPending
                                                                }
                                                                onClick={() =>
                                                                    setMode.mutate(
                                                                        {
                                                                            id: server.id,
                                                                            tool: tool.name,
                                                                            mode,
                                                                        },
                                                                    )
                                                                }
                                                            >
                                                                {label}
                                                            </Button>
                                                        ),
                                                    )}
                                                </div>
                                            </li>
                                        ))}
                                    </ul>
                                )}
                                <div className="flex flex-wrap gap-2">
                                    {server.sign_in === "browser" && (
                                        <Button
                                            size="sm"
                                            aria-label={`Sign in to ${server.name}`}
                                            disabled={signIn.isPending}
                                            onClick={() =>
                                                signIn.mutate(server.id)
                                            }
                                        >
                                            {server.signed_in
                                                ? "Sign in again"
                                                : "Sign in"}
                                        </Button>
                                    )}
                                    <Button
                                        variant="outline"
                                        size="sm"
                                        aria-label={`Edit ${server.name}`}
                                        onClick={() => setEditing(server)}
                                    >
                                        Edit
                                    </Button>
                                    <Button
                                        variant="outline"
                                        size="sm"
                                        aria-label={`Reconnect ${server.name}`}
                                        disabled={reconnect.isPending}
                                        onClick={() =>
                                            reconnect.mutate(server.id)
                                        }
                                    >
                                        Reconnect
                                    </Button>
                                    <Button
                                        variant="outline"
                                        size="sm"
                                        aria-label={`Remove ${server.name}`}
                                        onClick={() => setRemoving(server)}
                                    >
                                        Remove
                                    </Button>
                                </div>
                            </li>
                        ))}
                    </ul>
                )}
                <div>
                    <Button onClick={() => setAdding(true)}>
                        Add a server
                    </Button>
                </div>
            </div>
            <Dialog
                open={adding}
                onOpenChange={(next: boolean) => setAdding(next)}
            >
                <DialogContent className="gap-0 p-0">
                    <div className="border-border border-b px-4 py-3">
                        <DialogTitle>Add an MCP server</DialogTitle>
                        <DialogDescription>
                            Its tools are yours alone, and only in the written
                            chat, never by voice.
                        </DialogDescription>
                    </div>
                    <DialogBody className="flex flex-col gap-3">
                        <AddForm
                            onAdded={(server) => {
                                setAdding(false);
                                void settle();
                                if (server.sign_in === "browser")
                                    signIn.mutate(server.id);
                            }}
                            onCancel={() => setAdding(false)}
                        />
                    </DialogBody>
                </DialogContent>
            </Dialog>
            <Dialog
                open={editing !== null}
                onOpenChange={(next: boolean) => {
                    if (!next) setEditing(null);
                }}
            >
                <DialogContent className="gap-0 p-0">
                    <div className="border-border border-b px-4 py-3">
                        <DialogTitle>Edit {editing?.name}</DialogTitle>
                    </div>
                    <DialogBody className="flex flex-col gap-3">
                        {editing && (
                            <EditForm
                                server={editing}
                                onSaved={() => {
                                    setEditing(null);
                                    void settle();
                                }}
                                onCancel={() => setEditing(null)}
                            />
                        )}
                    </DialogBody>
                </DialogContent>
            </Dialog>
            <Dialog
                open={removing !== null}
                onOpenChange={(next: boolean) => {
                    if (!next) setRemoving(null);
                }}
            >
                <DialogContent className="gap-0 p-0">
                    <div className="border-border border-b px-4 py-3">
                        <DialogTitle>Remove {removing?.name}?</DialogTitle>
                        <DialogDescription>
                            Niles forgets its address, its key or sign-in, and
                            its tools.
                        </DialogDescription>
                    </div>
                    <DialogBody className="flex justify-end gap-2">
                        <Button
                            variant="outline"
                            onClick={() => setRemoving(null)}
                        >
                            Keep it
                        </Button>
                        <Button
                            variant="destructive"
                            disabled={remove.isPending}
                            onClick={() =>
                                removing && remove.mutate(removing.id)
                            }
                        >
                            Remove
                        </Button>
                    </DialogBody>
                </DialogContent>
            </Dialog>
        </>
    );
}

function AddForm({
    onAdded,
    onCancel,
}: {
    onAdded: (server: PersonalServer) => void;
    onCancel: () => void;
}) {
    const [name, setName] = useState("");
    const [url, setUrl] = useState("");
    const [signIn, setSignIn] = useState<ServerSignIn>("key");
    const [header, setHeader] = useState("");
    const [key, setKey] = useState("");
    const add = useMutation({
        mutationFn: api.addMyServer,
        onSuccess: onAdded,
        gcTime: 0,
    });
    const ready =
        name.trim() !== "" &&
        url.trim() !== "" &&
        (signIn === "browser" || key !== "");
    return (
        <form
            className="flex flex-col gap-3"
            onSubmit={(e) => {
                e.preventDefault();
                if (!ready) return;
                add.mutate({
                    name: name.trim(),
                    url: url.trim(),
                    sign_in: signIn,
                    ...(signIn === "key" ? { header: header.trim(), key } : {}),
                });
            }}
        >
            <Input
                aria-label="Name"
                placeholder="Notion"
                maxLength={40}
                value={name}
                onChange={(e) => setName(e.target.value)}
            />
            <Input
                aria-label="Address"
                placeholder="https://mcp.example.com/mcp"
                value={url}
                onChange={(e) => setUrl(e.target.value)}
            />
            <div
                role="group"
                aria-label="How it signs in"
                className="flex flex-wrap gap-2"
            >
                <Button
                    type="button"
                    size="sm"
                    variant={signIn === "key" ? "secondary" : "outline"}
                    aria-pressed={signIn === "key"}
                    onClick={() => setSignIn("key")}
                >
                    With a key
                </Button>
                <Button
                    type="button"
                    size="sm"
                    variant={signIn === "browser" ? "secondary" : "outline"}
                    aria-pressed={signIn === "browser"}
                    onClick={() => setSignIn("browser")}
                >
                    Sign in on its page
                </Button>
            </div>
            {signIn === "key" ? (
                <>
                    <Input
                        aria-label="Key"
                        type="password"
                        autoComplete="off"
                        placeholder="Key or token"
                        value={key}
                        onChange={(e) => setKey(e.target.value)}
                    />
                    <Input
                        aria-label="Header"
                        autoComplete="off"
                        placeholder="Header (optional; Authorization: Bearer by default)"
                        value={header}
                        onChange={(e) => setHeader(e.target.value)}
                    />
                </>
            ) : (
                <p className="text-muted-foreground text-xs">
                    After you add it, your browser goes to the server's own page
                    to sign in, and comes back here.
                </p>
            )}
            {add.error && (
                <p className="text-destructive text-sm">{add.error.message}</p>
            )}
            <div className="flex gap-2">
                <Button type="submit" disabled={!ready || add.isPending}>
                    Add
                </Button>
                <Button type="button" variant="outline" onClick={onCancel}>
                    Cancel
                </Button>
            </div>
        </form>
    );
}

function EditForm({
    server,
    onSaved,
    onCancel,
}: {
    server: PersonalServer;
    onSaved: () => void;
    onCancel: () => void;
}) {
    const [name, setName] = useState(server.name);
    const [header, setHeader] = useState(server.header ?? "");
    // Write-only: never filled from the server.
    const [key, setKey] = useState("");
    const save = useMutation({
        mutationFn: () =>
            api.updateMyServer(server.id, {
                name: name.trim(),
                header: header.trim(),
                ...(key ? { key } : {}),
            }),
        onSuccess: onSaved,
        gcTime: 0,
    });
    return (
        <form
            className="flex flex-col gap-3"
            onSubmit={(e) => {
                e.preventDefault();
                if (name.trim() !== "") save.mutate();
            }}
        >
            <Input
                aria-label="Name"
                maxLength={40}
                value={name}
                onChange={(e) => setName(e.target.value)}
            />
            {server.sign_in === "key" && (
                <>
                    <Input
                        aria-label="Key"
                        type="password"
                        autoComplete="off"
                        placeholder="Saved; leave empty to keep it"
                        value={key}
                        onChange={(e) => setKey(e.target.value)}
                    />
                    <Input
                        aria-label="Header"
                        autoComplete="off"
                        placeholder="Header (optional)"
                        value={header}
                        onChange={(e) => setHeader(e.target.value)}
                    />
                </>
            )}
            {save.error && (
                <p className="text-destructive text-sm">{save.error.message}</p>
            )}
            <div className="flex gap-2">
                <Button type="submit" disabled={save.isPending}>
                    Save
                </Button>
                <Button type="button" variant="outline" onClick={onCancel}>
                    Cancel
                </Button>
            </div>
        </form>
    );
}
