import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { SecretField } from "@/components/SecretField";
import type { SecretsReport } from "@/lib/api";

export interface SecretsCardProps {
  report: SecretsReport;
  onChanged: () => void;
}

/**
 * Every credential Niles holds, in one list.
 *
 * Overlaps with the integration cards on purpose: a key belongs to the
 * thing it connects to, which is where you set it up, and it also
 * belongs to a list of everything Niles is holding, which is what you
 * want when you are auditing or rotating. Same field either way — see
 * [`SecretField`] for why it never shows a value back.
 */
export function SecretsCard({ report, onChanged }: SecretsCardProps) {
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
          <SecretField
            key={secret.key}
            secret={secret}
            writable={report.writable}
            onChanged={onChanged}
          />
        ))}
      </CardContent>
    </Card>
  );
}
