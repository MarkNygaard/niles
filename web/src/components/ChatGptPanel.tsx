import { useState } from "react";
import { ExternalLink } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { api } from "@/lib/api";
import type { ChatGptStatus } from "@/lib/api";
import { cn } from "@/lib/utils";

export interface ChatGptPanelProps {
  enabled: boolean;
  model: string;
  /** Whether it is signed in, and as whom. Absent while it loads. */
  status?: ChatGptStatus;
  saving?: boolean;
  onEnabled: (enabled: boolean) => void;
  onModel: (model: string) => void;
  /** Signed in or out: the status and the keys behind it have changed. */
  onChanged: () => void;
}

/** What a ChatGPT subscription offers through Codex, quickest first, as
    Claude's are. Only as current as the pi version in the image: pi's
    own list decides which of them a subscription can reach. */
export const MODELS = [
  { id: "gpt-5.6-luna", label: "GPT-5.6 Luna", hint: "Quickest" },
  { id: "gpt-5.6-terra", label: "GPT-5.6 Terra", hint: "Balanced" },
  { id: "gpt-5.6-sol", label: "GPT-5.6 Sol", hint: "Stronger" },
  { id: "gpt-6-astra", label: "GPT-6 Astra", hint: "Most capable" },
  { id: "gpt-5.5", label: "GPT-5.5", hint: "Previous generation" },
];

/**
 * ChatGPT, answering in Niles's place on the household's own ChatGPT
 * subscription — everything Claude Code does, for a house that pays for
 * ChatGPT instead.
 *
 * Signing in is a paste-back: OpenAI sends the browser on to an address
 * on the computer the Codex CLI would be running on, which is not this
 * one, so that page fails to load and its address is what Niles needs.
 * The steps say so before it happens, because a page that fails to load
 * looks like something having gone wrong.
 */
export function ChatGptPanel({
  enabled,
  model,
  status,
  saving,
  onEnabled,
  onModel,
  onChanged,
}: ChatGptPanelProps) {
  const [signInUrl, setSignInUrl] = useState<string | null>(null);
  const [pasted, setPasted] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function run(work: () => Promise<void>) {
    setError(null);
    setBusy(true);
    try {
      await work();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  function signIn() {
    // Opened now, while this is still the click: a tab opened after the
    // request comes back is one a phone's browser blocks.
    const tab = window.open("about:blank", "_blank");
    void run(async () => {
      try {
        const { authorize_url } = await api.chatgptSignIn();
        setSignInUrl(authorize_url);
        if (tab) {
          tab.opener = null;
          tab.location.href = authorize_url;
        }
      } catch (e) {
        tab?.close();
        throw e;
      }
    });
  }

  function connect() {
    void run(async () => {
      await api.chatgptFinish(pasted.trim());
      setPasted("");
      setSignInUrl(null);
      onChanged();
    });
  }

  function signOut() {
    void run(async () => {
      await api.chatgptSignOut();
      onChanged();
    });
  }

  return (
    <div className="flex flex-col gap-4">
      <p className="text-muted-foreground text-sm">
        Answers the chat and new rules with ChatGPT, using Niles&apos;s own tools.
        Other spoken commands keep the faster voice model.
      </p>

      {/* Off and on without losing the sign-in or the model: Remove is
          for not wanting it at all. */}
      <label className="flex items-center justify-between gap-4">
        <span className="min-w-0">
          <span className="block text-sm font-medium">Answer with ChatGPT</span>
          <span className="text-muted-foreground block text-xs">
            Off, the chat uses the voice model.
          </span>
        </span>
        <Switch checked={enabled} disabled={saving} onCheckedChange={(next) => onEnabled(next)} />
      </label>

      <div>
        <div className="text-muted-foreground mb-1.5 text-xs">Model</div>
        {/* Buttons, as Claude's are, in rows: five do not fit on one
            line of a phone, and the hint is the reason to pick one. */}
        <div
          className="bg-muted/60 grid grid-cols-2 gap-1 rounded-lg p-1 sm:grid-cols-3"
          role="radiogroup"
          aria-label="Model"
        >
          {MODELS.map((m) => {
            const active = m.id === model;
            return (
              <button
                key={m.id}
                type="button"
                role="radio"
                aria-checked={active}
                disabled={saving}
                onClick={() => !active && onModel(m.id)}
                className={cn(
                  "flex flex-col items-center rounded-md px-1 py-1.5 text-sm transition-colors",
                  "focus-visible:ring-2 focus-visible:ring-ring/50 focus-visible:outline-none",
                  active
                    ? "bg-background text-foreground shadow-sm"
                    : "text-muted-foreground hover:text-foreground",
                )}
              >
                {m.label}
                <span className="text-muted-foreground text-[11px]">{m.hint}</span>
              </button>
            );
          })}
        </div>
      </div>

      <div className="flex flex-col gap-3 border-t pt-4">
        {status?.signed_in ? (
          <div className="flex items-center justify-between gap-3">
            <p className="min-w-0 text-sm">
              {status.email ? (
                <>
                  Signed in as <span className="font-medium break-all">{status.email}</span>
                </>
              ) : (
                "Signed in to ChatGPT"
              )}
            </p>
            <Button variant="outline" size="sm" disabled={busy} onClick={signOut}>
              Sign out
            </Button>
          </div>
        ) : status && !status.writable ? (
          <p className="text-muted-foreground text-sm">
            Signing in needs Niles&apos;s database and its secrets key, to keep the
            sign-in somewhere safe.
          </p>
        ) : (
          <>
            <p className="text-sm font-medium">Not signed in</p>
            <ol className="text-muted-foreground flex list-decimal flex-col gap-1.5 pl-5 text-sm">
              <li>
                Sign in with the ChatGPT account that has the subscription.
              </li>
              <li>
                After signing in, the page that opens will fail to load. That is
                expected: copy its address from the address bar and paste it here.
              </li>
            </ol>
            <div>
              <Button disabled={busy || !status} onClick={signIn}>
                <ExternalLink aria-hidden /> Sign in with ChatGPT
              </Button>
            </div>
            {signInUrl && (
              // For a browser that would not open the tab.
              <a
                href={signInUrl}
                target="_blank"
                rel="noopener noreferrer"
                className="text-muted-foreground text-xs underline underline-offset-2"
              >
                Nothing opened? Open the sign-in page
              </a>
            )}
            <div className="flex gap-2">
              <Input
                aria-label="Address of the page that failed to load"
                placeholder="http://localhost:1455/auth/callback?code=…"
                autoComplete="off"
                value={pasted}
                onChange={(e) => setPasted(e.target.value)}
              />
              <Button disabled={busy || pasted.trim() === ""} onClick={connect}>
                Connect
              </Button>
            </div>
          </>
        )}
        {error && <p className="text-destructive text-sm">{error}</p>}
      </div>

      <p className="text-muted-foreground text-xs">
        Needs a paid ChatGPT plan. Niles keeps the sign-in and renews it itself.
      </p>
    </div>
  );
}
