import { useState } from "react";
import { Check, Trash2 } from "lucide-react";
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
import type { SecretsReport } from "@/lib/api";

export interface SecretsCardProps {
  report: SecretsReport;
  onChanged: () => void;
}

/**
 * The credentials, typed in rather than mounted.
 *
 * Write-only by design: there is no route that hands one back, so the
 * page can say whether something is set and never what it is. A field
 * that could show you a broker password is a field that could show it
 * to whoever is standing behind you.
 *
 * A secret already coming from an environment variable is left alone —
 * it is set, it works, and offering to replace it from here would
 * quietly move where it lives.
 */
export function SecretsCard({ report, onChanged }: SecretsCardProps) {
  const [drafts, setDrafts] = useState<Record<string, string>>({});
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  async function save(key: string) {
    const value = drafts[key]?.trim();
    if (!value) return;
    setError(null);
    setBusy(key);
    try {
      await api.setSecret(key, value);
      setDrafts((d) => ({ ...d, [key]: "" }));
      onChanged();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  }

  async function clear(key: string) {
    setError(null);
    setBusy(key);
    try {
      await api.clearSecret(key);
      onChanged();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle>Credentials</CardTitle>
        <CardDescription>
          Kept encrypted in Niles's own database. They are never shown again
          after saving — this page can tell you that something is set, not what
          it is.
        </CardDescription>
      </CardHeader>
      <CardContent className="flex flex-col gap-4">
        {!report.writable && (
          <p className="text-muted-foreground text-sm">
            Niles has no database or no encryption key, so these can only come
            from environment variables. Set <code>[database]</code> and{" "}
            <code>NILES_SECRET_KEY</code> to fill them in here instead.
          </p>
        )}

        {report.secrets.map((secret) => (
          <div key={secret.key} className="flex flex-col gap-1.5">
            <div className="flex items-baseline justify-between gap-3">
              <span className="text-sm font-medium">{secret.label}</span>
              {secret.set && (
                <span className="text-muted-foreground flex items-center gap-1 text-xs">
                  <Check aria-hidden className="text-lit size-3" />
                  {secret.stored ? "saved here" : "from the environment"}
                </span>
              )}
            </div>

            {secret.stored || !secret.set ? (
              <div className="flex items-center gap-2">
                <Input
                  type="password"
                  autoComplete="off"
                  aria-label={secret.label}
                  placeholder={secret.set ? "Replace it…" : "Not set"}
                  value={drafts[secret.key] ?? ""}
                  disabled={!report.writable || busy === secret.key}
                  onChange={(e) =>
                    setDrafts((d) => ({ ...d, [secret.key]: e.target.value }))
                  }
                  onKeyDown={(e) => {
                    if (e.key === "Enter") save(secret.key);
                  }}
                />
                <Button
                  variant="outline"
                  disabled={
                    !report.writable ||
                    busy === secret.key ||
                    !drafts[secret.key]?.trim()
                  }
                  onClick={() => save(secret.key)}
                >
                  Save
                </Button>
                {secret.stored && (
                  <Button
                    variant="ghost"
                    aria-label={`Clear ${secret.label}`}
                    disabled={busy === secret.key}
                    onClick={() => clear(secret.key)}
                  >
                    <Trash2 aria-hidden />
                  </Button>
                )}
              </div>
            ) : (
              /* Set in the environment: it works, and offering to
                 replace it here would quietly move where it lives. */
              <p className="text-muted-foreground text-xs">
                Coming from an environment variable. Remove it there to manage
                it here instead.
              </p>
            )}
          </div>
        ))}

        {error && <p className="text-destructive text-sm">{error}</p>}
      </CardContent>
    </Card>
  );
}
