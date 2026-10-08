import { SecretField } from "@/components/SecretField";
import { Switch } from "@/components/ui/switch";
import type { Secret } from "@/lib/api";

export interface NemligPanelProps {
  enabled: boolean;
  username?: Secret;
  password?: Secret;
  writable: boolean;
  saving?: boolean;
  onEnabled: (enabled: boolean) => void;
  onSecretsChanged: () => void;
}

/**
 * The household's nemlig.com account.
 *
 * A switch as well as Remove, because the login is a thing to keep:
 * off for a fortnight's holiday should not mean typing the password in
 * again afterwards.
 */
export function NemligPanel({
  enabled,
  username,
  password,
  writable,
  saving,
  onEnabled,
  onSecretsChanged,
}: NemligPanelProps) {
  return (
    <div className="flex flex-col gap-4">
      <p className="text-muted-foreground text-sm">
        Each item on the shopping list gets a button to choose the product at
        nemlig.com — with the picture and the price — and the choice is kept for
        next time. Paying always happens at nemlig.com.
      </p>

      <label className="flex items-center justify-between gap-4">
        <span className="min-w-0">
          <span className="block text-sm font-medium">Use nemlig.com</span>
          <span className="text-muted-foreground block text-xs">
            Off keeps the login, and the list works as before.
          </span>
        </span>
        <Switch checked={enabled} disabled={saving} onCheckedChange={(next) => onEnabled(next)} />
      </label>

      {username && (
        <SecretField secret={username} writable={writable} onChanged={onSecretsChanged} />
      )}
      {password && (
        <SecretField secret={password} writable={writable} onChanged={onSecretsChanged} />
      )}

      <p className="text-muted-foreground text-xs">
        nemlig.com has no public API. Niles talks to it the way its own website
        does, so it can stop working when they change the site — the shopping
        list carries on regardless.
      </p>
    </div>
  );
}
