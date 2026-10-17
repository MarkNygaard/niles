import { SecretField } from "@/components/SecretField";
import type { Secret } from "@/lib/api";

/** Where Linear sends its webhook, on this Niles. */
export function linearWebhookUrl(origin = window.location.origin): string {
  return `${origin}/webhooks/linear`;
}

export interface LinearWebhookPanelProps {
  secret: Secret;
  writable: boolean;
  onSecretsChanged: () => void;
}

/**
 * Linear telling Niles when an issue moves, which Niles then says out
 * loud. Beside Linear's key because it is the same add-on, though the
 * key is for asking Linear and this is for being told.
 *
 * Saving the secret is what turns the webhook on: until then the
 * address answers as if it were not there.
 */
export function LinearWebhookPanel({ secret, writable, onSecretsChanged }: LinearWebhookPanelProps) {
  return (
    <div className="flex flex-col gap-3 border-t pt-4">
      <div className="flex flex-col gap-1">
        <span className="text-sm font-medium">Say when an issue moves</span>
        <span className="text-muted-foreground text-xs">
          Niles says it in every room with a satellite when an issue goes to review or is done.
        </span>
      </div>
      <ol className="text-muted-foreground list-decimal space-y-1 pl-4 text-xs">
        <li>
          In Linear, open Settings → API → Webhooks and make a new one for <b>Issues</b>, for the
          teams you want to hear about.
        </li>
        <li>
          Send it to <code className="font-mono break-all">{linearWebhookUrl()}</code>. Linear has
          to be able to reach that address.
        </li>
        <li>Copy its signing secret into the field below.</li>
      </ol>
      <SecretField secret={secret} writable={writable} onChanged={onSecretsChanged} />
    </div>
  );
}
