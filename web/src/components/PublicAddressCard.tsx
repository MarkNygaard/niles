import { useEffect, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { api, valueAt } from "@/lib/api";
import { publicUrlProblem } from "@/lib/plugins";

/**
 * `[public] url`: where the internet reaches this Niles, if it does.
 *
 * Only add-ons read it, for the webhook and sign-in addresses they give
 * a service; nothing else in Niles changes when it is set, and setting
 * it exposes nothing. Making the address reachable is the household's
 * own doing, at their router or tunnel.
 */
export function PublicAddressCard() {
  const queryClient = useQueryClient();
  const config = useQuery({ queryKey: ["config"], queryFn: api.getConfig });
  const saved = valueAt(config.data?.effective, "public.url");
  const current = typeof saved === "string" ? saved : "";
  const [draft, setDraft] = useState(current);
  useEffect(() => setDraft(current), [current]);

  const settle = () => {
    queryClient.invalidateQueries({ queryKey: ["config"] });
    queryClient.invalidateQueries({ queryKey: ["plugins"] });
  };
  const save = useMutation({
    mutationFn: (url: string) =>
      url.trim() === ""
        ? api.resetPath("public.url")
        : api.patchConfig({ public: { url: url.trim() } }),
    onSuccess: settle,
  });

  const problem = publicUrlProblem(draft);
  const changed = draft.trim() !== current;

  return (
    <Card>
      <CardHeader>
        <CardTitle>Public address</CardTitle>
        <CardDescription>
          Where the internet reaches this Niles, if it does: <code>https://home.example.dk</code>.
          Add-ons build their webhook and sign-in addresses on it. Setting it opens nothing;
          only what your router or tunnel lets through is reachable.
        </CardDescription>
      </CardHeader>
      <CardContent className="flex flex-col gap-2">
        <form
          className="flex gap-2"
          onSubmit={(e) => {
            e.preventDefault();
            if (!problem && changed) save.mutate(draft);
          }}
        >
          <Input
            aria-label="Public address"
            placeholder="https://home.example.dk"
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            aria-invalid={problem ? true : undefined}
            className="min-w-0 flex-1"
          />
          <Button type="submit" disabled={!changed || !!problem || save.isPending}>
            Save
          </Button>
        </form>
        {problem && <p className="text-destructive text-sm">{problem}</p>}
        {save.isError && (
          <p role="alert" className="text-destructive text-sm">
            {save.error.message}
          </p>
        )}
      </CardContent>
    </Card>
  );
}
