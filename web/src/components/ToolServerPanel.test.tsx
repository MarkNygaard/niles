import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { ToolServerPanel, connection } from "@/components/ToolServerPanel";
import type { ToolServer } from "@/lib/api";

const LINEAR: ToolServer = {
  id: "linear",
  status: "working",
  message: null,
  tools: [
    {
      name: "list_issues",
      title: "List issues",
      description: "List issues in a team.",
      read_only: true,
      destructive: false,
    },
    {
      name: "delete_issue",
      title: null,
      description: "Delete an issue.",
      read_only: false,
      destructive: true,
    },
  ],
};

function setup(props: Partial<React.ComponentProps<typeof ToolServerPanel>> = {}) {
  const onChange = vi.fn();
  render(
    <ToolServerPanel
      id="linear"
      server={LINEAR}
      modes={{}}
      notes=""
      writable
      onChange={onChange}
      onSecretsChanged={vi.fn()}
      onRefreshed={vi.fn()}
      {...props}
    />,
  );
  return { onChange };
}

describe("ToolServerPanel", () => {
  it("lists every tool the server offers, on unless switched off", () => {
    setup({ modes: { delete_issue: "off" } });
    expect(screen.getByRole("switch", { name: "Let Niles use List issues" })).toBeChecked();
    expect(screen.getByRole("switch", { name: "Let Niles use delete_issue" })).not.toBeChecked();
    expect(screen.getByRole("status")).toHaveTextContent("Connected. 1 of 2 tools on.");
  });

  it("shows what the server says each tool does", () => {
    setup();
    expect(screen.getByText("Reads only")).toBeInTheDocument();
    expect(screen.getByText("Can delete")).toBeInTheDocument();
  });

  it("writes a tool's setting under the server's name for it", () => {
    const { onChange } = setup();
    fireEvent.click(screen.getByRole("switch", { name: "Let Niles use delete_issue" }));
    expect(onChange).toHaveBeenCalledWith([
      { path: "tool_servers.linear.tools.delete_issue", value: "off" },
    ]);
  });

  it("writes the notes when you leave them", () => {
    const { onChange } = setup();
    const notes = screen.getByRole("textbox", { name: "Notes for Niles" });
    fireEvent.change(notes, { target: { value: "Coding tasks go to Todo. " } });
    expect(onChange).not.toHaveBeenCalled();
    fireEvent.blur(notes);
    expect(onChange).toHaveBeenCalledWith([
      { path: "tool_servers.linear.notes", value: "Coding tasks go to Todo." },
    ]);
  });
});

describe("connection", () => {
  it("says why it is not connected, in the server's words", () => {
    expect(connection({ ...LINEAR, status: "failed", message: "401: the key was refused" }, 0, 0)).toBe(
      "Not connected: 401: the key was refused",
    );
    expect(connection({ ...LINEAR, status: "needs_key" }, 0, 0)).toBe(
      "Add its API key to connect.",
    );
    expect(connection(undefined, 0, 0)).toBe("Not connected yet.");
  });
});
