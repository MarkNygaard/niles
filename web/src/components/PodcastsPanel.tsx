import { useCallback, useEffect, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { CommitField } from "@/components/CommitField";
import { SecretField } from "@/components/SecretField";
import { Button } from "@/components/ui/button";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { api } from "@/lib/api";
import type { Secret } from "@/lib/api";
import {
  DEFAULT_REGION,
  DEFAULT_VOICES,
  kokoroAddress,
  keepDays,
  r2Endpoint,
  statusLine,
  testLines,
  voiceChoices,
  voiceLabel,
} from "@/lib/podcasts";
import type { PodcastsSettings, Probe, Tone } from "@/lib/podcasts";
import { cn } from "@/lib/utils";

export interface PodcastsPanelProps {
  /** The `integrations.podcasts` section as the config has it, undefined
      until the add-on is added. */
  settings: PodcastsSettings | undefined;
  accessKeyId?: Secret;
  secretAccessKey?: Secret;
  writable: boolean;
  saving?: boolean;
  onChange: (entries: { path: string; value: unknown }[]) => void;
  onSecretsChanged: () => void;
}

const TONE_CLASS: Record<Tone, string> = {
  off: "text-muted-foreground",
  pending: "text-muted-foreground",
  ok: "",
  bad: "text-destructive",
};

/**
 * Podcasts: Niles researches a topic and records a two-host episode,
 * voiced by Kokoro and kept in a private Cloudflare R2 bucket.
 *
 * Nothing is made until the add-on is added and complete, and whether
 * it works is said only by the test, through the server: what the
 * browser can reach says nothing about what Niles can. The config is
 * read for every episode, so changing it or removing the add-on takes
 * hold without a restart.
 */
export function PodcastsPanel({
  settings,
  accessKeyId,
  secretAccessKey,
  writable,
  saving,
  onChange,
  onSecretsChanged,
}: PodcastsPanelProps) {
  const [probe, setProbe] = useState<Probe>({ state: "idle" });
  const [daysProblem, setDaysProblem] = useState<string | null>(null);

  const kokoro = settings?.kokoro_url?.trim() ?? "";
  const endpoint = settings?.endpoint?.trim() ?? "";
  const bucket = settings?.bucket ?? "";
  const firstVoice = settings?.first_voice ?? DEFAULT_VOICES[0];
  const secondVoice = settings?.second_voice ?? DEFAULT_VOICES[1];

  const voices = useQuery({
    queryKey: ["podcasts", "voices", kokoro],
    queryFn: () => api.podcastVoices(),
    enabled: kokoro !== "",
  });

  const run = useCallback(() => {
    setProbe({ state: "running" });
    api
      .testPodcasts()
      .then((result) => setProbe({ state: "done", result }))
      .catch((e: unknown) =>
        setProbe({ state: "failed", message: e instanceof Error ? e.message : String(e) }),
      );
  }, []);

  // As it opens, when there is something to test with. Again for new
  // values: the last one's answer says nothing of them.
  useEffect(() => {
    if (kokoro && endpoint && bucket.trim()) run();
  }, [kokoro, endpoint, bucket, run]);

  const status = statusLine(probe);

  const voicePicker = (which: "first" | "second") => {
    const label = which === "first" ? "First host" : "Second host";
    const current = which === "first" ? firstVoice : secondVoice;
    const other = which === "first" ? secondVoice : firstVoice;
    return (
      <label className="flex min-w-0 flex-1 flex-col gap-1">
        <span className="text-muted-foreground text-xs">{label}</span>
        <Select
          value={current}
          disabled={saving}
          onValueChange={(next: string | null) => {
            if (next) onChange([{ path: `integrations.podcasts.${which}_voice`, value: next }]);
          }}
        >
          <SelectTrigger aria-label={label} className="w-full">
            {/* The value is the id; the box says the name. */}
            <SelectValue>{(value: string) => voiceLabel(value)}</SelectValue>
          </SelectTrigger>
          <SelectContent>
            {voiceChoices(voices.data?.voices ?? [], current, other).map((id) => (
              <SelectItem key={id} value={id}>
                {voiceLabel(id)}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </label>
    );
  };

  return (
    <div className="flex flex-col gap-4">
      <p className="text-muted-foreground text-sm">
        Ask Niles to research a topic, by voice or in the app. A few minutes later you get a short
        episode, at most 8 minutes, of two hosts talking it through, with the sources listed
        underneath. Listen in the app or on a Sonos speaker. It's free, fast, and the scripts are
        voiced without leaving the house. Niles does the research with its web search, writes the
        two-host script, and then records it itself, line by line, in each host's voice.
      </p>
      <p className="text-muted-foreground text-sm">
        The recommended setup is Kokoro, which you run yourself, and a private Cloudflare R2 bucket
        with an access key for that bucket only.
      </p>
      <p className="text-muted-foreground text-sm">
        Episodes are one MP3 each in the bucket, named <code>podcasts/…</code>, and a document under
        Saved. A lifecycle rule of about 30 days on the bucket is only a backstop: Niles deletes
        each episode itself when its time is up.
      </p>

      <CommitField
        label="Kokoro address"
        value={kokoro}
        placeholder="http://kokoro.ns.svc:8880"
        disabled={saving}
        onCommit={(typed) =>
          onChange([{ path: "integrations.podcasts.kokoro_url", value: kokoroAddress(typed) }])
        }
      />

      {kokoro && (
        <div className="flex flex-col gap-2">
          <div className="flex flex-wrap gap-3">
            {voicePicker("first")}
            {voicePicker("second")}
          </div>
          {voices.data?.error && <p className="text-destructive text-sm">{voices.data.error}</p>}
        </div>
      )}

      <CommitField
        label="R2 endpoint"
        value={endpoint}
        placeholder="https://<account-id>.r2.cloudflarestorage.com"
        disabled={saving}
        onCommit={(typed) =>
          onChange([{ path: "integrations.podcasts.endpoint", value: r2Endpoint(typed) }])
        }
      />
      <div className="flex flex-wrap gap-3">
        <CommitField
          label="Bucket"
          value={bucket}
          placeholder="niles-podcasts"
          disabled={saving}
          onCommit={(typed) =>
            onChange([
              { path: "integrations.podcasts.bucket", value: typed.trim().toLowerCase() },
            ])
          }
        />
        <CommitField
          label="Region"
          value={settings?.region ?? DEFAULT_REGION}
          placeholder={DEFAULT_REGION}
          disabled={saving}
          onCommit={(typed) =>
            onChange([{ path: "integrations.podcasts.region", value: typed.trim() }])
          }
        />
      </div>
      <p className="text-muted-foreground -mt-3 text-xs">auto for Cloudflare R2.</p>

      {accessKeyId && (
        <SecretField secret={accessKeyId} writable={writable} onChanged={onSecretsChanged} />
      )}
      {secretAccessKey && (
        <SecretField secret={secretAccessKey} writable={writable} onChanged={onSecretsChanged} />
      )}

      <div className="flex flex-col gap-1">
        <CommitField
          label="Keep episodes for (days)"
          value={settings?.keep_days?.toString() ?? ""}
          placeholder="14"
          disabled={saving}
          onCommit={(typed) => {
            const days = keepDays(typed);
            if (days === null) {
              setDaysProblem("A number of days from 1 to 365.");
              return;
            }
            setDaysProblem(null);
            onChange([{ path: "integrations.podcasts.keep_days", value: days }]);
          }}
        />
        {daysProblem && <p className="text-destructive text-sm">{daysProblem}</p>}
      </div>

      <div className="flex flex-col gap-2">
        <div className="flex items-center justify-between gap-3">
          <p role="status" className={cn("min-w-0 text-sm", TONE_CLASS[status.tone])}>
            {status.text}
          </p>
          <Button
            variant="outline"
            size="sm"
            disabled={saving || probe.state === "running"}
            onClick={run}
          >
            {probe.state === "running" ? "Testing…" : "Test"}
          </Button>
        </div>
        {probe.state === "done" && (
          <ul aria-label="Test results" className="text-muted-foreground space-y-1 text-xs">
            {testLines(probe.result).map((line) => (
              <li key={line}>{line}</li>
            ))}
          </ul>
        )}
      </div>

      <p className="text-muted-foreground text-xs">
        The test says a one-second sample in each host's voice through Kokoro and checks it is MP3,
        then uploads a tiny file, reads it back the way the app and Sonos do, and deletes it.
        Nothing of the answer names your address or keys.
      </p>
    </div>
  );
}
