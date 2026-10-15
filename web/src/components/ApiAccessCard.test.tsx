import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { ApiAccessCard } from "@/components/ApiAccessCard";
import { api } from "@/lib/api";

const KEY = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

function renderCard() {
  render(
    <QueryClientProvider
      client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}
    >
      <ApiAccessCard />
    </QueryClientProvider>,
  );
}

describe("ApiAccessCard", () => {
  const writeText = vi.fn();

  beforeEach(() => {
    vi.restoreAllMocks();
    writeText.mockReset().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", {
      value: { writeText },
      configurable: true,
    });
  });

  afterEach(() => {
    Object.defineProperty(navigator, "clipboard", { value: undefined, configurable: true });
  });

  it("keeps the key masked until the eye is pressed", async () => {
    vi.spyOn(api, "apiToken").mockResolvedValue({ source: "stored", token: KEY, writable: true });
    renderCard();

    const shown = await screen.findByLabelText("API key");
    expect(shown).not.toHaveTextContent(KEY);
    expect(screen.queryByText(KEY)).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "Show the key" }));
    expect(screen.getByLabelText("API key")).toHaveTextContent(KEY);

    fireEvent.click(screen.getByRole("button", { name: "Hide the key" }));
    expect(screen.queryByText(KEY)).toBeNull();
  });

  it("copies the key, masked or not, and says so", async () => {
    vi.spyOn(api, "apiToken").mockResolvedValue({ source: "stored", token: KEY, writable: true });
    renderCard();

    fireEvent.click(await screen.findByRole("button", { name: "Copy the key" }));

    await waitFor(() => expect(writeText).toHaveBeenCalledWith(KEY));
    expect(await screen.findByText("Copied")).toBeInTheDocument();
  });

  it("asks before making a new key, then shows the new one", async () => {
    vi.spyOn(api, "apiToken").mockResolvedValue({ source: "stored", token: KEY, writable: true });
    const rotate = vi
      .spyOn(api, "rotateApiToken")
      .mockResolvedValue({ source: "stored", token: "the-new-key", writable: true });
    renderCard();

    fireEvent.click(await screen.findByRole("button", { name: "New key" }));
    expect(await screen.findByText("Scripts using the old key will stop working.")).toBeInTheDocument();
    expect(rotate).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: "Make a new key" }));

    await waitFor(() => expect(rotate).toHaveBeenCalledTimes(1));
    // Shown straight away: it is made to be copied somewhere.
    await waitFor(() =>
      expect(screen.getByLabelText("API key")).toHaveTextContent("the-new-key"),
    );
  });

  it("keeping the old key changes nothing", async () => {
    vi.spyOn(api, "apiToken").mockResolvedValue({ source: "stored", token: KEY, writable: true });
    const rotate = vi.spyOn(api, "rotateApiToken");
    renderCard();

    fireEvent.click(await screen.findByRole("button", { name: "New key" }));
    fireEvent.click(await screen.findByRole("button", { name: "Keep this one" }));

    expect(rotate).not.toHaveBeenCalled();
  });

  it("says where a key from the environment is set, and offers nothing to do with it", async () => {
    vi.spyOn(api, "apiToken").mockResolvedValue({
      source: "environment",
      env_var: "NILES_API_TOKEN",
      writable: true,
    });
    renderCard();

    expect(await screen.findByText("NILES_API_TOKEN")).toBeInTheDocument();
    expect(screen.getByText(/change it there/)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Show the key" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Copy the key" })).toBeNull();
    expect(screen.queryByRole("button", { name: "New key" })).toBeNull();
  });

  it("explains that a key needs a database when there is none", async () => {
    vi.spyOn(api, "apiToken").mockResolvedValue({ source: "none", writable: false });
    renderCard();

    expect(await screen.findByText(/needs a database/)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /key/ })).toBeNull();
  });

  it("shows how to call this Niles with it", async () => {
    vi.spyOn(api, "apiToken").mockResolvedValue({ source: "stored", token: KEY, writable: true });
    renderCard();

    expect(
      await screen.findByText(
        `curl -H "Authorization: Bearer <token>" ${window.location.origin}/devices`,
      ),
    ).toBeInTheDocument();
  });
});
