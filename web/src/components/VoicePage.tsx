import type { ReactNode } from "react";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Switch } from "@/components/ui/switch";
import { RoleCard } from "@/components/RoleCard";
import type { Role, RoleChange } from "@/components/RoleCard";
import { SetupChecklist } from "@/components/SetupChecklist";
import { cn } from "@/lib/utils";
import type { Integration, SecretsReport, SetupGap } from "@/lib/api";

/** `integrations.assistant`: which one answers, or neither. */
export type Assistant = "none" | "claude_code" | "chatgpt";

/** One role as the config has it. */
export interface RoleState {
  provider?: string;
  model?: string;
  baseUrl?: string;
  effort?: string;
}

export interface VoicePageProps {
  /** Everything in the catalogue; the providers are picked out here. */
  integrations: Integration[];
  stt: RoleState;
  llm: RoleState;
  /** Niles's own models, per role, for when none is written down. */
  defaultModels: Partial<Record<Role, string>>;
  secrets?: SecretsReport;
  /** The setup gaps this page answers. */
  gaps: SetupGap[];
  requireName: boolean;
  /** The language row, drawn by the page that owns rows. */
  language: ReactNode;
  /** Piper's rows, likewise. */
  speaking: ReactNode;
  /** What `integrations.assistant` says, if anything. */
  assistant?: string;
  claudeCode?: { enabled?: boolean };
  chatgpt?: { enabled?: boolean };
  chatgptSignedIn?: boolean;
  saving?: boolean;
  error?: string;
  onSaveRole: (role: Role, change: RoleChange) => void;
  onRequireName: (on: boolean) => void;
  onAssistant: (value: Assistant) => void;
}

/**
 * Voice & AI: who hears, who thinks, who speaks, and which assistant
 * answers. The base system, by the job each part does.
 */
export function VoicePage({
  integrations,
  stt,
  llm,
  defaultModels,
  secrets,
  gaps,
  requireName,
  language,
  speaking,
  assistant,
  claudeCode,
  chatgpt,
  chatgptSignedIn,
  saving,
  error,
  onSaveRole,
  onRequireName,
  onAssistant,
}: VoicePageProps) {
  const providers = integrations.filter((i) => i.kind === "provider");
  const options = assistantOptions({ integrations, claudeCode, chatgpt, secrets, chatgptSignedIn });

  return (
    <div className="flex flex-col gap-4">
      <SetupChecklist gaps={gaps} />

      <Card>
        <CardHeader>
          <CardTitle>Hearing</CardTitle>
          <CardDescription>
            Turns what a satellite heard into words.
          </CardDescription>
        </CardHeader>
        <CardContent className="divide-border flex flex-col gap-5 divide-y">
          <RoleCard
            role="stt"
            title="Hearing"
            description="Pick who listens, and give it its key."
            providers={providers}
            current={stt.provider}
            model={stt.model ?? ""}
            fallbackHost={hostOf(stt.baseUrl)}
            defaultModel={defaultModels.stt}
            secrets={secrets}
            saving={saving}
            onSave={(change) => onSaveRole("stt", change)}
          />
          <label className="flex items-center justify-between gap-4 pt-5">
            <span className="min-w-0">
              <span className="block text-sm font-medium">
                Only when it hears its name
              </span>
              <span className="text-muted-foreground block text-xs">
                Ignore a wake unless what was heard starts with &ldquo;Niles&rdquo;
                &mdash; the television that set the satellite off almost never
                says it. Best with ElevenLabs, which spells the name right.
              </span>
            </span>
            <Switch
              aria-label="Only when it hears its name"
              checked={requireName}
              disabled={saving}
              onCheckedChange={(next: boolean) => onRequireName(next)}
            />
          </label>
          {language}
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Thinking</CardTitle>
          <CardDescription>
            Answers anything a pattern cannot, while you wait.
          </CardDescription>
        </CardHeader>
        <CardContent>
          <RoleCard
            role="llm"
            title="Thinking"
            description="Pick who answers, give it its key, and say how hard it thinks."
            providers={providers}
            current={llm.provider}
            model={llm.model ?? ""}
            effort={llm.effort}
            fallbackHost={hostOf(llm.baseUrl)}
            defaultModel={defaultModels.llm}
            secrets={secrets}
            saving={saving}
            onSave={(change) => onSaveRole("llm", change)}
          />
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Speaking</CardTitle>
          <CardDescription>
            What a satellite says back is spoken by Piper, which runs in the
            house rather than in somebody else&apos;s cloud.
          </CardDescription>
        </CardHeader>
        <CardContent className="divide-border divide-y">{speaking}</CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Assistant</CardTitle>
          <CardDescription>
            Who answers the chat, works out new rules and reviews the day.
            Claude and ChatGPT run on your own subscription, and are set up
            under Add-ons, in Assistants &amp; tools.
          </CardDescription>
        </CardHeader>
        <CardContent>
          <AssistantChoice
            options={options}
            value={assistantShown(assistant, options)}
            disabled={saving}
            onChoose={onAssistant}
          />
        </CardContent>
      </Card>

      {error && <p className="text-destructive text-sm">{error}</p>}
    </div>
  );
}

/** The host out of a base URL, for the line saying what answers today. */
function hostOf(baseUrl?: string): string | undefined {
  const rest = baseUrl?.split("://")[1];
  return rest ? rest.split(/[/?]/)[0] || undefined : undefined;
}

/** What the assistant choice is worked out from. */
export interface AssistantInputs {
  integrations: Integration[];
  claudeCode?: { enabled?: boolean };
  chatgpt?: { enabled?: boolean };
  secrets?: SecretsReport;
  /** Whether ChatGPT is signed in, when Niles has said. */
  chatgptSignedIn?: boolean;
}

/**
 * Who can be chosen to answer: Niles's own model always, and Claude or
 * ChatGPT once each is set up — added, switched on, and with its
 * token or sign-in. Offering one that cannot run would be choosing
 * something that quietly falls back.
 */
export function assistantOptions(inputs: AssistantInputs): { id: Assistant; label: string }[] {
  const added = (id: string) => inputs.integrations.some((i) => i.id === id && i.added);
  // A key the report does not list is one this page cannot judge, so
  // it is not held against the assistant.
  const has = (key: string) =>
    inputs.secrets?.secrets.find((s) => s.key === key)?.source !== "unset";
  const claude =
    added("claude_code") &&
    inputs.claudeCode?.enabled !== false &&
    has("integrations.claude_code.oauth_token");
  const chatgpt =
    added("chatgpt") &&
    inputs.chatgpt?.enabled !== false &&
    (inputs.chatgptSignedIn ?? has("integrations.chatgpt.access_token"));
  return [
    { id: "none", label: "Niles's own model" },
    ...(claude ? [{ id: "claude_code" as const, label: "Claude" }] : []),
    ...(chatgpt ? [{ id: "chatgpt" as const, label: "ChatGPT" }] : []),
  ];
}

/**
 * Which option reads as chosen.
 *
 * What answers, rather than only what is written: unset means Claude,
 * and a preferred assistant that is not set up gives way to the other
 * one, exactly as Niles itself decides. With neither, Niles's own model
 * answers whatever is written.
 */
export function assistantShown(
  preference: string | undefined,
  options: { id: Assistant }[],
): Assistant {
  if (preference === "none") return "none";
  const wanted = preference === "chatgpt" ? "chatgpt" : "claude_code";
  if (options.some((o) => o.id === wanted)) return wanted;
  return options.find((o) => o.id !== "none")?.id ?? "none";
}

const ASSISTANT_HINTS: Record<Assistant, string> = {
  none: "Niles's own model answers the chat and handles new rules, and nothing runs in the background.",
  claude_code: "Claude answers the chat, works out new rules and reviews the day.",
  chatgpt: "ChatGPT answers the chat, works out new rules and reviews the day.",
};

function AssistantChoice({
  options,
  value,
  disabled,
  onChoose,
}: {
  options: { id: Assistant; label: string }[];
  value: Assistant;
  disabled?: boolean;
  onChoose: (value: Assistant) => void;
}) {
  return (
    <div className="flex flex-col gap-2">
      <div
        role="radiogroup"
        aria-label="Which assistant answers"
        className="bg-muted/60 flex w-fit flex-wrap gap-1 rounded-lg p-1"
      >
        {options.map((option) => {
          const active = option.id === value;
          return (
            <button
              key={option.id}
              type="button"
              role="radio"
              aria-checked={active}
              disabled={disabled}
              onClick={() => !active && onChoose(option.id)}
              className={cn(
                "rounded-md px-3 py-1 text-sm transition-colors",
                "focus-visible:ring-2 focus-visible:ring-ring/50 focus-visible:outline-none",
                active
                  ? "bg-background text-foreground shadow-sm"
                  : "text-muted-foreground hover:text-foreground",
              )}
            >
              {option.label}
            </button>
          );
        })}
      </div>
      <p className="text-muted-foreground text-xs">{ASSISTANT_HINTS[value]}</p>
      {options.length === 1 && (
        <p className="text-muted-foreground text-xs">
          Set up Claude Code or ChatGPT under Add-ons to have one answer instead.
        </p>
      )}
    </div>
  );
}

/** The `[[providers]]` array as it stands, ready to be written back. */
export function providerRows(integrations: Integration[]) {
  return integrations.filter((i) => i.added && i.kind === "provider").map(asRow);
}

function asRow(integration: Integration) {
  return {
    name: integration.id,
    base_url: integration.base_url,
    serves: integration.serves,
  };
}

/**
 * The config writes one Save on a role card makes, as one revision.
 *
 * Only what changed. A provider not declared yet is declared — that is
 * what makes picking it the whole of setting it up — by appending, so
 * the others stay. The key is not here: it goes to the secret store,
 * after this has declared the provider it belongs to.
 */
export function roleEntries(
  role: Role,
  change: RoleChange,
  was: RoleState,
  integrations: Integration[],
): { path: string; value: unknown }[] {
  const entries: { path: string; value: unknown }[] = [];
  const declared = providerRows(integrations);
  const adding = integrations.find(
    (i) => i.id === change.provider && i.kind === "provider" && !i.added,
  );
  if (adding) {
    entries.push({ path: "providers", value: [...declared, asRow(adding)] });
  }
  if ((change.provider ?? null) !== (was.provider ?? null)) {
    entries.push({ path: `${role}.provider`, value: change.provider ?? null });
  }
  if (change.model !== (was.model ?? "")) {
    entries.push({ path: `${role}.model`, value: change.model });
  }
  if (role === "llm" && (change.effort ?? null) !== (was.effort ?? null)) {
    entries.push({ path: "llm.reasoning_effort", value: change.effort ?? null });
  }
  return entries;
}
