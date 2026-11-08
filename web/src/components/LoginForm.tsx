import { useState } from "react";
import { useMutation } from "@tanstack/react-query";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { api } from "@/lib/api";
import type { LoginDraft, SavedLogin } from "@/lib/api";
import { originOf, sitesOf } from "@/lib/logins";

/**
 * The one form a saved login is written in, wherever it was asked
 * from: the Logins page, or a card's ask in the chat. It posts straight
 * to /me/logins; the values never go through the chat.
 */
export function LoginForm({
  login,
  asked,
  onSaved,
  onCancel,
}: {
  login?: SavedLogin;
  /** A card's ask: its label is prefilled and its site is fixed. */
  asked?: { label: string; site: string };
  onSaved: (login: SavedLogin) => void;
  onCancel?: () => void;
}) {
  const [label, setLabel] = useState(login?.label ?? asked?.label ?? "");
  const [sites, setSites] = useState(login?.origins.join("\n") ?? "");
  // A card's ask holds its site fixed; any others the person names go here.
  const [more, setMore] = useState("");
  const [username, setUsername] = useState(login?.username ?? "");
  // Always empty at first: the password is write-only, never filled
  // from the server.
  const [password, setPassword] = useState("");
  // The draft holds the password: it goes with the form, not kept for five minutes.
  const save = useMutation({
    mutationFn: (draft: LoginDraft) =>
      login ? api.updateLogin(login.id, draft) : api.addLogin(draft),
    onSuccess: onSaved,
    gcTime: 0,
  });
  const origins = asked ? [asked.site, ...sitesOf(more)] : sitesOf(sites);
  const ready =
    label.trim() !== "" &&
    origins.length > 0 &&
    username.trim() !== "" &&
    (login !== undefined || password !== "");
  // A line Niles will keep differently, so the person sees the normal
  // form before saving: an IDN reads as `xn--…`.
  const previews = sitesOf(asked ? more : sites)
    .map((line, i) => ({ key: `${i}:${line}`, line, origin: originOf(line) }))
    .filter((p) => p.origin !== null && p.origin !== p.line.trim());

  return (
    <form
      className="flex flex-col gap-3"
      onSubmit={(e) => {
        e.preventDefault();
        if (!ready) return;
        save.mutate({
          label: label.trim(),
          origins,
          username: username.trim(),
          ...(password ? { password } : {}),
        });
      }}
    >
      <Input
        aria-label="Label"
        autoComplete="off"
        placeholder="Shop"
        maxLength={60}
        value={label}
        onChange={(e) => setLabel(e.target.value)}
      />
      {asked ? (
        <div className="flex flex-col gap-2">
          <span className="text-sm font-medium">Site</span>
          <code
            aria-label="Site"
            className="bg-muted text-foreground rounded-md px-3 py-2 font-mono text-xs break-all select-all"
          >
            {asked.site}
          </code>
          <p className="text-muted-foreground text-xs">
            Check it is the site you mean: Niles uses this login there and nowhere else.
          </p>
          <Textarea
            aria-label="Other sites"
            placeholder="Another address it signs in on, one per line"
            value={more}
            onChange={(e) => setMore(e.target.value)}
          />
        </div>
      ) : (
        <div className="flex flex-col gap-1">
          <Textarea
            aria-label="Sites"
            placeholder="https://shop.example"
            value={sites}
            onChange={(e) => setSites(e.target.value)}
          />
          <p className="text-muted-foreground text-xs">
            One per line. Add another when the site signs in on a different address.
          </p>
        </div>
      )}
      {previews.map((p) => (
        <p key={p.key} className="text-muted-foreground text-xs">
          Saved as {p.origin}
        </p>
      ))}
      <Input
        aria-label="Username"
        autoComplete="off"
        placeholder="Username or email"
        value={username}
        onChange={(e) => setUsername(e.target.value)}
      />
      <Input
        aria-label="Password"
        type="password"
        autoComplete="off"
        placeholder={login ? "Saved; leave empty to keep it" : "Password"}
        value={password}
        onChange={(e) => setPassword(e.target.value)}
      />
      {save.error && <p className="text-destructive text-sm">{save.error.message}</p>}
      <div className="flex gap-2">
        <Button type="submit" disabled={!ready || save.isPending}>
          Save
        </Button>
        {onCancel && (
          <Button type="button" variant="outline" onClick={onCancel}>
            Cancel
          </Button>
        )}
      </div>
    </form>
  );
}
