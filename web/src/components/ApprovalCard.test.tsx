import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { ApprovalCard, outcome } from "@/components/ApprovalCard";
import type { ApprovalCard as Card } from "@/lib/api";

const ISSUE: Card = {
  id: "a1",
  title: "Let Linear create issue?",
  subtitle: "Linear",
  icon: "linear",
  fields: [
    { label: "Title", value: "WLED presets" },
    { label: "Description", value: "x".repeat(300) },
  ],
  body: null,
  allow_conversation: true,
  status: "pending",
  outcome: null,
};

const MAIL: Card = {
  ...ISSUE,
  id: "m1",
  title: "Send this email?",
  subtitle: "From your Gmail, mark@gmail.com",
  icon: "mail",
  fields: [
    { label: "To", value: "jakob@example.com" },
    { label: "Subject", value: "Running late" },
  ],
  body: "Hi Jakob,\n\nI'll be fifteen minutes late.",
  allow_conversation: false,
};

describe("ApprovalCard", () => {
  it("asks, shows what would happen, and offers the three answers", () => {
    const onDecide = vi.fn();
    render(<ApprovalCard card={ISSUE} onDecide={onDecide} />);
    expect(screen.getByRole("heading", { name: "Let Linear create issue?" })).toBeInTheDocument();
    expect(screen.getByText("WLED presets")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Allow for this conversation" }));
    expect(onDecide).toHaveBeenCalledWith("conversation");
    fireEvent.click(screen.getByRole("button", { name: "Deny" }));
    expect(onDecide).toHaveBeenCalledWith("deny");
  });

  it("draws a mail as a mail, and only ever allows it once", () => {
    const onDecide = vi.fn();
    render(<ApprovalCard card={MAIL} onDecide={onDecide} />);
    expect(screen.getByText("jakob@example.com")).toBeInTheDocument();
    expect(screen.getByText(/fifteen minutes late/)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Allow for this conversation" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Allow once" }));
    expect(onDecide).toHaveBeenCalledWith("once");
  });

  it("says what became of it instead of the buttons once answered", () => {
    render(<ApprovalCard card={{ ...ISSUE, status: "lapsed" }} onDecide={vi.fn()} />);
    expect(screen.queryByRole("button", { name: "Allow once" })).toBeNull();
    expect(screen.getByRole("status")).toHaveTextContent("Lapsed. Nothing was done.");
  });

  it("says why an allowed call failed", () => {
    expect(outcome({ ...ISSUE, status: "failed", outcome: "401: the key was refused" })).toBe(
      "Allowed, but it didn't work: 401: the key was refused",
    );
    expect(outcome({ ...ISSUE, status: "allowed", outcome: "Done" })).toBe("Allowed. Done.");
  });
});
