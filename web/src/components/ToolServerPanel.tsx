import { useState } from "react";
import { RefreshCw } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import { SecretField } from "@/components/SecretField";
import { api } from "@/lib/api";
import type { RemoteTool, Secret, ToolServer } from "@/lib/api";

export type ToolMode = "on" | "off";

export interface ToolServerPanelProps {
  id: string;
  /** What the server said, once Niles has asked. */
  server?: ToolServer;
  /** Each tool's setting as the config has it; a tool not here is on. */
  modes: Record<string, ToolMode>;
  notes: string;
  /** What the notes say before anybody has written any. */
  notesPlaceholder?: string;
  secret?: Secret;
  writable: boolean;
  saving?: boolean;
  onChange: (entries: { path: string; value: unknown }[]) => void;
  onSecretsChanged: () => void;
  /** Ask the server again; the list says when it has answered. */
  onRefreshed: () => void;
}

/**
 * A remote MCP server: its key, what it offers, and which of it the
 * model may use.
 *
 * Every tool starts on, because the model does what it is told. The
 * server's own word on each — reads only, or deletes — is shown beside
 * the switch, as the thing somebody choosing would want to know.
 */
export function ToolServerPanel({
  id,
  server,
  modes,
  notes,
  notesPlaceholder,
  secret,
  writable,
  saving,
  onChange,
  onSecretsChanged,
  onRefreshed,
}: ToolServerPanelProps) {
  const [refreshing, setRefreshing] = useState(false);
  const tools = [...(server?.tools ?? [])].sort((a, b) => a.name.localeCompare(b.name));
  const on = tools.filter((t) => (modes[t.name] ?? "on") === "on").length;

  async function refresh() {
    setRefreshing(true);
    try {
      await api.refreshToolServer(id);
    } finally {
      setRefreshing(false);
      onRefreshed();
    }
  }

  return (
    <div className="flex flex-col gap-4">
      {secret && (
        <SecretField
          secret={secret}
          writable={writable}
          onChanged={onSecretsChanged}
        />
      )}

      <div className="flex items-center justify-between gap-3">
        <p role="status" className="text-muted-foreground text-sm">
          {connection(server, tools.length, on)}
        </p>
        <Button
          variant="outline"
          size="sm"
          disabled={refreshing || !server}
          onClick={refresh}
        >
          <RefreshCw aria-hidden /> Refresh
        </Button>
      </div>

      <NotesField
        value={notes}
        placeholder={notesPlaceholder}
        disabled={saving}
        onCommit={(value) => onChange([{ path: `tool_servers.${id}.notes`, value }])}
      />

      {tools.length > 0 && (
        <ul className="flex flex-col divide-y rounded-lg border">
          {tools.map((tool) => (
            <ToolRow
              key={tool.name}
              tool={tool}
              on={(modes[tool.name] ?? "on") === "on"}
              disabled={saving}
              onChange={(next) =>
                onChange([
                  { path: `tool_servers.${id}.tools.${tool.name}`, value: next ? "on" : "off" },
                ])
              }
            />
          ))}
        </ul>
      )}
    </div>
  );
}

/** One line on where the connection stands. */
export function connection(server: ToolServer | undefined, tools: number, on: number): string {
  if (!server) return "Not connected yet.";
  switch (server.status) {
    case "connecting":
      return "Connecting…";
    case "needs_key":
      return "Add its API key to connect.";
    case "failed":
      return `Not connected: ${server.message ?? "it did not answer"}`;
    case "working":
      return `Connected. ${on} of ${tools} tools on.`;
  }
}

function ToolRow({
  tool,
  on,
  disabled,
  onChange,
}: {
  tool: RemoteTool;
  on: boolean;
  disabled?: boolean;
  onChange: (on: boolean) => void;
}) {
  const label = tool.title ?? tool.name;
  return (
    <li className="flex items-start justify-between gap-3 px-3 py-2">
      <div className="min-w-0">
        <div className="flex flex-wrap items-center gap-1.5">
          <span className="text-sm font-medium">{label}</span>
          {tool.read_only && <Badge variant="secondary">Reads only</Badge>}
          {tool.destructive && <Badge variant="outline">Can delete</Badge>}
        </div>
        {tool.description && (
          <p className="text-muted-foreground line-clamp-2 text-xs">{tool.description}</p>
        )}
      </div>
      <Switch
        aria-label={`Let Niles use ${label}`}
        checked={on}
        disabled={disabled}
        onCheckedChange={(next) => onChange(next)}
      />
    </li>
  );
}

/** Written when you leave it, like every other field on these cards. */
function NotesField({
  value,
  placeholder,
  disabled,
  onCommit,
}: {
  value: string;
  placeholder?: string;
  disabled?: boolean;
  onCommit: (value: string) => void;
}) {
  const [draft, setDraft] = useState(value);
  const [seen, setSeen] = useState(value);
  if (seen !== value) {
    setSeen(value);
    setDraft(value);
  }
  return (
    <label className="flex flex-col gap-1">
      <span className="text-muted-foreground text-xs">
        Notes for Niles, given with these tools
      </span>
      <Textarea
        value={draft}
        aria-label="Notes for Niles"
        placeholder={placeholder}
        disabled={disabled}
        onChange={(e) => setDraft(e.target.value)}
        onBlur={() => {
          const next = draft.trim();
          if (next !== value) onCommit(next);
        }}
      />
    </label>
  );
}
