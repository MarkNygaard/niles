import { afterEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { LoginsPanel } from "@/components/LoginsPanel";
import { ApiError, api } from "@/lib/api";
import type { SavedLogin } from "@/lib/api";

/** The fixture's password: it goes to addLogin, and never comes back. */
const PASSWORD = "hunter2-pw";

const SHOP: SavedLogin = {
  id: 1,
  label: "Shop",
  origins: ["https://shop.example", "login.shop.example"],
  username: "majse",
  last_used: null,
  signed_in: true,
};

const BANK: SavedLogin = {
  id: 2,
  label: "Bank",
  origins: ["https://bank.example"],
  username: "majse",
  last_used: new Date(Date.now() - 3 * 60 * 60_000).toISOString(),
  signed_in: false,
};

function renderPanel() {
  render(
    <QueryClientProvider
      client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}
    >
      <LoginsPanel />
    </QueryClientProvider>,
  );
}

afterEach(() => {
  vi.restoreAllMocks();
});

describe("LoginsPanel", () => {
  it("lists each login with its sites, username and whether it is signed in, and never a password", async () => {
    vi.spyOn(api, "logins").mockResolvedValue({ logins: [SHOP, BANK] });
    renderPanel();

    const list = await screen.findByRole("list", { name: "Your logins" });
    expect(list).toHaveTextContent("Shop");
    expect(list).toHaveTextContent("https://shop.example · login.shop.example");
    expect(list).toHaveTextContent("majse");
    expect(screen.getByText("Signed in")).toBeInTheDocument();
    expect(list).toHaveTextContent("Bank");
    expect(list).toHaveTextContent("Last used 3 hours ago");

    // The password never reaches the page: no field holds it and no
    // text says it.
    expect(screen.queryByDisplayValue(PASSWORD)).toBeNull();
    expect(screen.queryByText(PASSWORD)).toBeNull();
  });

  it("adds a login with its sites one per line, and Save waits for all four", async () => {
    vi.spyOn(api, "logins").mockResolvedValue({ logins: [] });
    const add = vi.spyOn(api, "addLogin").mockResolvedValue(SHOP);
    renderPanel();

    fireEvent.click(await screen.findByRole("button", { name: "Add login" }));
    const save = screen.getByRole("button", { name: "Save" });
    expect(save).toBeDisabled();

    fireEvent.change(screen.getByLabelText("Label"), { target: { value: "Shop" } });
    expect(save).toBeDisabled();
    fireEvent.change(screen.getByLabelText("Sites"), {
      target: { value: "https://shop.example\nlogin.shop.example" },
    });
    fireEvent.change(screen.getByLabelText("Username"), { target: { value: "majse" } });
    fireEvent.change(screen.getByLabelText("Password"), { target: { value: "pw" } });
    expect(save).toBeEnabled();

    fireEvent.click(save);
    await waitFor(() => expect(add).toHaveBeenCalledTimes(1));
    expect(add).toHaveBeenCalledWith({
      label: "Shop",
      origins: ["https://shop.example", "login.shop.example"],
      username: "majse",
      password: "pw",
    });
  });

  it("edits with the password field empty, and an empty password keeps it", async () => {
    vi.spyOn(api, "logins").mockResolvedValue({ logins: [SHOP] });
    const update = vi.spyOn(api, "updateLogin").mockResolvedValue(SHOP);
    renderPanel();

    fireEvent.click(await screen.findByRole("button", { name: "Edit Shop" }));
    expect(screen.getByLabelText("Username")).toHaveValue("majse");
    const password = screen.getByLabelText("Password");
    expect(password).toHaveValue("");
    expect(password).toHaveAttribute("placeholder", "Saved; leave empty to keep it");

    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(update).toHaveBeenCalledTimes(1));
    expect(update).toHaveBeenCalledWith(1, {
      label: "Shop",
      origins: ["https://shop.example", "login.shop.example"],
      username: "majse",
    });
    // A blank password sends no password key at all: the saved one
    // stays, and nothing empty overwrites it.
    expect(update.mock.calls[0][1]).not.toHaveProperty("password");
  });

  it("removes only after asking", async () => {
    vi.spyOn(api, "logins").mockResolvedValue({ logins: [SHOP] });
    const remove = vi.spyOn(api, "removeLogin").mockResolvedValue(undefined);
    renderPanel();

    fireEvent.click(await screen.findByRole("button", { name: "Remove Shop" }));
    expect(
      await screen.findByText("Niles forgets its username and password, and the site's cookies with them."),
    ).toBeInTheDocument();
    expect(remove).not.toHaveBeenCalled();

    fireEvent.click(
      screen
        .getAllByRole("button", { name: "Remove" })
        .find((b) => b.closest("[role=dialog]"))!,
    );
    await waitFor(() => expect(remove).toHaveBeenCalledTimes(1));
    expect(remove.mock.calls[0][0]).toBe(1);
  });

  it("offers Sign out only for a signed-in login", async () => {
    vi.spyOn(api, "logins").mockResolvedValue({ logins: [SHOP, BANK] });
    const signOut = vi.spyOn(api, "signOutLogin").mockResolvedValue(undefined);
    renderPanel();

    fireEvent.click(await screen.findByRole("button", { name: "Sign out of Shop" }));
    await waitFor(() => expect(signOut).toHaveBeenCalledTimes(1));
    expect(signOut.mock.calls[0][0]).toBe(1);
    expect(screen.queryByRole("button", { name: "Sign out of Bank" })).toBeNull();
  });

  it("says why when Niles keeps no logins", async () => {
    vi.spyOn(api, "logins").mockRejectedValue(
      new ApiError(
        "Saved logins need Niles's database and its secrets key, and this Niles doesn't have both.",
        501,
      ),
    );
    renderPanel();

    expect(
      await screen.findByText(
        "Saved logins need Niles's database and its secrets key, and this Niles doesn't have both.",
      ),
    ).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Add login" })).toBeNull();
  });
});
