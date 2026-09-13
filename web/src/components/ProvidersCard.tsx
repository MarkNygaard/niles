import { useState } from "react";
import { Plus, Trash2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import type { Provider } from "@/lib/api";

export interface ProvidersCardProps {
  providers: Provider[];
  saving?: boolean;
  error?: string;
  onChange: (providers: Provider[]) => void;
}

/**
 * The accounts Niles has with inference providers.
 *
 * One account, however many things use it: a single Groq key serves
 * speech-to-text and the language model, and used to be entered twice
 * because each section carried its own copy of the endpoint.
 *
 * Adding one here does not switch anything over. Which provider a role
 * uses is chosen beside that role's model, because the two always
 * change together — a model name is not portable between providers,
 * and picking one here while the model stayed put would fail at the
 * next request rather than on this page.
 */
export function ProvidersCard({
  providers,
  saving,
  error,
  onChange,
}: ProvidersCardProps) {
  const [name, setName] = useState("");
  const [baseUrl, setBaseUrl] = useState("");
  const [problem, setProblem] = useState<string | null>(null);

  function add(event: React.FormEvent) {
    event.preventDefault();
    const trimmed = name.trim().toLowerCase();
    const url = baseUrl.trim();
    if (!trimmed || !url) return;
    // The server checks both. Saying it here means the answer arrives
    // while the thing being described is still on screen.
    if (providers.some((p) => p.name === trimmed)) {
      setProblem(`There is already a provider called ${trimmed}.`);
      return;
    }
    if (!url.startsWith("http://") && !url.startsWith("https://")) {
      setProblem("The endpoint has to be a URL, starting http:// or https://");
      return;
    }
    setProblem(null);
    onChange([...providers, { name: trimmed, base_url: url }]);
    setName("");
    setBaseUrl("");
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle>Providers</CardTitle>
        <CardDescription>
          Where Niles sends speech and language. One account can serve both —
          its key lives under Credentials, once, rather than once per use.
        </CardDescription>
      </CardHeader>
      <CardContent className="flex flex-col gap-4">
        {providers.length === 0 && (
          <p className="text-muted-foreground text-sm">
            None yet. Speech and language fall back to whatever the config
            already names, which works but keeps a copy of the key per use.
          </p>
        )}

        {providers.map((provider) => (
          <div
            key={provider.name}
            className="bg-muted/40 flex items-center gap-3 rounded-lg px-3 py-2"
          >
            <span className="min-w-0 flex-1">
              <span className="block truncate text-sm font-medium">
                {provider.name}
              </span>
              <span className="text-muted-foreground block truncate font-mono text-xs">
                {provider.base_url}
              </span>
            </span>
            <Button
              variant="ghost"
              aria-label={`Remove ${provider.name}`}
              disabled={saving}
              onClick={() =>
                onChange(providers.filter((p) => p.name !== provider.name))
              }
            >
              <Trash2 aria-hidden />
            </Button>
          </div>
        ))}

        <form onSubmit={add} className="flex flex-col gap-2 sm:flex-row">
          <Input
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder="groq"
            aria-label="Provider name"
            className="sm:w-40"
          />
          <Input
            value={baseUrl}
            onChange={(e) => setBaseUrl(e.target.value)}
            placeholder="https://api.groq.com/openai/v1"
            aria-label="Provider endpoint"
            className="font-mono sm:flex-1"
          />
          <Button type="submit" variant="outline" disabled={saving}>
            <Plus aria-hidden /> Add
          </Button>
        </form>

        {(problem || error) && (
          <p className="text-destructive text-sm">{problem ?? error}</p>
        )}
      </CardContent>
    </Card>
  );
}
