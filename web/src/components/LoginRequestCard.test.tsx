import { afterEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { LoginRequestCard } from "@/components/LoginRequestCard";
import { api } from "@/lib/api";
import type { LoginRequest } from "@/lib/api";

const REQUEST: LoginRequest = {
  id: "r1",
  label: "Shop",
  site: "https://xn--bcher-kva.example",
  status: "waiting",
  login: null,
};

const SAVED = {
  id: 42,
  label: "Shop",
  origins: ["https://xn--bcher-kva.example"],
  username: "majse",
  last_used: null,
  signed_in: false,
};

function renderCard(props: Partial<Parameters<typeof LoginRequestCard>[0]> = {}) {
  const onAdded = vi.fn();
  render(
    <QueryClientProvider
      client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}
    >
      <LoginRequestCard request={REQUEST} onAdded={onAdded} {...props} />
    </QueryClientProvider>,
  );
  return onAdded;
}

afterEach(() => {
  vi.restoreAllMocks();
});

describe("LoginRequestCard", () => {
  it("asks in the label's name, with the whole site", () => {
    renderCard();
    const card = screen.getByRole("region", { name: "Shop wants a login" });
    expect(card).toHaveTextContent("https://xn--bcher-kva.example");
  });

  it("opens the Logins form with the site fixed and the label filled in", async () => {
    renderCard();
    fireEvent.click(screen.getByRole("button", { name: "Add login" }));

    const site = await screen.findByLabelText("Site");
    expect(site).toHaveTextContent("https://xn--bcher-kva.example");
    expect(screen.getByLabelText("Label")).toHaveValue("Shop");
    expect(screen.getByText(/It goes to Niles alone, never into the chat\./)).toBeInTheDocument();
  });

  it("saves through /me/logins and tells the chat only the new login's id", async () => {
    const add = vi.spyOn(api, "addLogin").mockResolvedValue(SAVED);
    const onAdded = renderCard();
    fireEvent.click(screen.getByRole("button", { name: "Add login" }));

    fireEvent.change(await screen.findByLabelText("Username"), { target: { value: "majse" } });
    fireEvent.change(screen.getByLabelText("Password"), { target: { value: "hunter2" } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() =>
      expect(add).toHaveBeenCalledWith({
        label: "Shop",
        origins: ["https://xn--bcher-kva.example"],
        username: "majse",
        password: "hunter2",
      }),
    );
    await waitFor(() => expect(onAdded).toHaveBeenCalledWith(42));
    // Ids only cross into the chat: neither value appears anywhere in
    // what the card hands it.
    const calls = JSON.stringify(onAdded.mock.calls);
    expect(calls).not.toContain("majse");
    expect(calls).not.toContain("hunter2");
  });

  it("an added card says so and offers nothing", () => {
    renderCard({ request: { ...REQUEST, status: "added", login: 42 } });
    expect(screen.getByRole("status")).toHaveTextContent("Login added");
    expect(screen.queryByRole("button", { name: "Add login" })).toBeNull();
  });
});
