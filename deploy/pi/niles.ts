import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

// Niles's tools, for pi, which speaks no MCP.
//
// Niles serves its tools over MCP on a loopback port, for Claude Code. This
// is the other end of that for pi: it lists them once when pi starts and
// calls them there one by one. There is one implementation of every tool,
// and the run's token decides which of them this run may see and on whose
// behalf they act — the person typing, or the background review with its
// own few tools. Nothing here decides that.
//
// pi loads this with `-e`, with discovery off, so nothing else on disk can
// add a tool. Its built-in tools are switched off on the command line.

const URL = process.env.NILES_MCP_URL;
const TOKEN = process.env.NILES_MCP_TOKEN;

// Long: a tool can search the web or fill a grocery basket. The timeout is
// there so a hung call fails that call rather than the whole run.
const TIMEOUT_MS = 600_000;

interface McpTool {
  name: string;
  description?: string;
  inputSchema?: unknown;
}

interface CallResult {
  content?: Array<{ type: string; text?: string }>;
  isError?: boolean;
}

let nextId = 1;

/** One JSON-RPC request, one POST: the shape Niles's server answers. */
async function rpc<T>(method: string, params: unknown, signal?: AbortSignal): Promise<T> {
  const timeout = AbortSignal.timeout(TIMEOUT_MS);
  const response = await fetch(URL as string, {
    method: "POST",
    headers: {
      "content-type": "application/json",
      authorization: `Bearer ${TOKEN}`,
    },
    body: JSON.stringify({ jsonrpc: "2.0", id: nextId++, method, params }),
    signal: signal ? AbortSignal.any([signal, timeout]) : timeout,
  });
  if (!response.ok) {
    throw new Error(`Niles answered ${response.status}`);
  }
  const body = (await response.json()) as { result?: T; error?: { message?: string } };
  if (body.error) {
    throw new Error(body.error.message ?? "Niles refused the request");
  }
  return body.result as T;
}

// pi awaits this, so the tools are registered before the model is asked
// anything.
export default async function (pi: ExtensionAPI) {
  if (!URL || !TOKEN) {
    // Nothing registered, rather than tools that always fail: a model
    // offered a tool it cannot use reports the failure as though the house
    // were broken.
    console.error("[niles] no NILES_MCP_URL or NILES_MCP_TOKEN; Niles's tools are unavailable.");
    return;
  }

  let tools: McpTool[];
  try {
    tools = (await rpc<{ tools?: McpTool[] }>("tools/list", {})).tools ?? [];
  } catch (err) {
    console.error(`[niles] could not list the tools: ${err instanceof Error ? err.message : err}`);
    return;
  }

  for (const tool of tools) {
    pi.registerTool({
      name: tool.name,
      label: tool.name,
      description: tool.description ?? "",
      // Passed through as it is: the schema the model sees is the one the
      // tool is written against, not a copy that can disagree with it.
      parameters: (tool.inputSchema ?? { type: "object", properties: {} }) as never,
      async execute(_toolCallId, params, signal) {
        const result = await rpc<CallResult>(
          "tools/call",
          { name: tool.name, arguments: params ?? {} },
          signal,
        );
        const text = (result.content ?? [])
          .filter((block) => block.type === "text")
          .map((block) => block.text ?? "")
          .join("\n");
        // Thrown, so pi marks the result as an error and the model can
        // try something else.
        if (result.isError) throw new Error(text || `${tool.name} failed`);
        return { content: [{ type: "text" as const, text }], details: undefined };
      },
    });
  }
}
