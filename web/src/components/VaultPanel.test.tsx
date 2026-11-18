import { afterEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { VaultPanel, byDay, matching } from "@/components/VaultPanel";
import { ApiError, api } from "@/lib/api";
import type { LoginActivity, SavedLogin } from "@/lib/api";
import * as qr from "@/lib/qr";

/** The fixture's password: it goes to addLogin, and never comes back. */
const PASSWORD = "hunter2-pw";

const SHOP: SavedLogin = {
  id: 1,
  label: "Shop",
  origins: ["https://shop.example", "login.shop.example"],
  username: "majse",
  last_used: null,
  signed_in: true,
  authenticator: true,
};

const BANK: SavedLogin = {
  id: 2,
  label: "Bank",
  origins: ["https://bank.example"],
  username: "majse.bank",
  last_used: new Date(Date.now() - 3 * 60 * 60_000).toISOString(),
  signed_in: false,
  authenticator: false,
};

function renderPanel() {
  render(
    <QueryClientProvider
      client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}
    >
      <VaultPanel />
    </QueryClientProvider>,
  );
}

/** Opens a login from the list, as a tap on a phone does. */
async function open(label: string) {
  const list = await screen.findByRole("list", { name: "Your logins" });
  fireEvent.click(within(list).getByText(label));
  return screen.findByRole("region", { name: label });
}

afterEach(() => {
  vi.restoreAllMocks();
});

describe("VaultPanel", () => {
  it("lists each login by its label and username, and never a password", async () => {
    vi.spyOn(api, "logins").mockResolvedValue({ logins: [SHOP, BANK] });
    vi.spyOn(api, "loginActivity").mockResolvedValue({ activity: [] });
    renderPanel();

    const list = await screen.findByRole("list", { name: "Your logins" });
    expect(list).toHaveTextContent("Shop");
    expect(list).toHaveTextContent("Bank");
    expect(list).toHaveTextContent("majse.bank");
    expect(screen.getByText("Encrypted in Niles · only you can use these")).toBeInTheDocument();

    const shop = await open("Shop");
    expect(shop).toHaveTextContent("shop.example · login.shop.example");
    expect(within(shop).getByText("Signed in")).toBeInTheDocument();
    expect(within(shop).getAllByText("Saved")).toHaveLength(2);
    // The password never reaches the page: no field holds it and no
    // text says it.
    expect(screen.queryByDisplayValue(PASSWORD)).toBeNull();
    expect(screen.queryByText(PASSWORD)).toBeNull();
    expect(screen.queryByRole("button", { name: /reveal|show the password|copy the password/i })).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "All logins" }));
    const bank = await open("Bank");
    expect(bank).toHaveTextContent("Not set");
    expect(bank).toHaveTextContent("Last used 3 hours ago");
  });

  it("searches by label, username or site", async () => {
    vi.spyOn(api, "logins").mockResolvedValue({ logins: [SHOP, BANK] });
    renderPanel();
    fireEvent.change(await screen.findByLabelText("Search logins"), {
      target: { value: "bank.example" },
    });
    const list = screen.getByRole("list", { name: "Your logins" });
    expect(list).toHaveTextContent("Bank");
    expect(list).not.toHaveTextContent("Shop");
    fireEvent.change(screen.getByLabelText("Search logins"), { target: { value: "nothing" } });
    expect(screen.getByText("No login matches that.")).toBeInTheDocument();
    expect(matching([SHOP, BANK], "  ")).toEqual([SHOP, BANK]);
  });

  it("shows the first login beside the list on a desk", async () => {
    vi.spyOn(window, "matchMedia").mockImplementation(
      (query: string) =>
        ({
          matches: query.includes("min-width"),
          media: query,
          addEventListener: () => {},
          removeEventListener: () => {},
        }) as unknown as MediaQueryList,
    );
    vi.spyOn(api, "logins").mockResolvedValue({ logins: [SHOP, BANK] });
    vi.spyOn(api, "loginActivity").mockResolvedValue({ activity: [] });
    renderPanel();
    expect(await screen.findByRole("region", { name: "Shop" })).toBeInTheDocument();
    expect(screen.getByRole("list", { name: "Your logins" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "All logins" })).toBeNull();
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
    vi.spyOn(api, "loginActivity").mockResolvedValue({ activity: [] });
    const update = vi.spyOn(api, "updateLogin").mockResolvedValue(SHOP);
    renderPanel();

    await open("Shop");
    fireEvent.click(screen.getByRole("button", { name: "Edit Shop" }));
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

  it("deletes only after asking", async () => {
    vi.spyOn(api, "logins").mockResolvedValue({ logins: [SHOP] });
    vi.spyOn(api, "loginActivity").mockResolvedValue({ activity: [] });
    const remove = vi.spyOn(api, "removeLogin").mockResolvedValue(undefined);
    renderPanel();

    await open("Shop");
    fireEvent.click(screen.getByRole("button", { name: "Remove Shop" }));
    expect(
      await screen.findByText(
        "Niles forgets its username, password and authenticator key, the site's cookies, and what it was used for.",
      ),
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

  it("takes an authenticator key, never shows it, and can remove it", async () => {
    vi.spyOn(api, "logins").mockResolvedValue({ logins: [SHOP, BANK] });
    vi.spyOn(api, "loginActivity").mockResolvedValue({ activity: [] });
    const update = vi.spyOn(api, "updateLogin").mockResolvedValue(SHOP);
    renderPanel();

    await open("Shop");
    fireEvent.click(screen.getByRole("button", { name: "Edit Shop" }));
    const key = screen.getByLabelText("Authenticator key");
    expect(key).toHaveValue("");
    expect(key).toHaveAttribute("type", "password");
    expect(key).toHaveAttribute("placeholder", "Saved; leave empty to keep it");

    fireEvent.click(screen.getByRole("button", { name: "Remove the authenticator key" }));
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(update).toHaveBeenCalledTimes(1));
    expect(update.mock.calls[0][1]).toMatchObject({ remove_authenticator: true });
    expect(update.mock.calls[0][1]).not.toHaveProperty("authenticator");
  });

  it("sends a new key with the login, and none when the field is empty", async () => {
    vi.spyOn(api, "logins").mockResolvedValue({ logins: [BANK] });
    vi.spyOn(api, "loginActivity").mockResolvedValue({ activity: [] });
    const update = vi.spyOn(api, "updateLogin").mockResolvedValue(BANK);
    renderPanel();

    await open("Bank");
    fireEvent.click(screen.getByRole("button", { name: "Edit Bank" }));
    expect(screen.queryByRole("button", { name: "Remove the authenticator key" })).toBeNull();
    fireEvent.change(screen.getByLabelText("Authenticator key"), {
      target: { value: " JBSWY3DPEHPK3PXP " },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(update).toHaveBeenCalledTimes(1));
    expect(update.mock.calls[0][1]).toMatchObject({ authenticator: "JBSWY3DPEHPK3PXP" });
    expect(update.mock.calls[0][1]).not.toHaveProperty("remove_authenticator");
  });

  it("reads the authenticator key from a QR code picture on this device", async () => {
    vi.spyOn(api, "logins").mockResolvedValue({ logins: [BANK] });
    vi.spyOn(api, "loginActivity").mockResolvedValue({ activity: [] });
    const link = "otpauth://totp/Bank:majse?secret=JBSWY3DPEHPK3PXP&issuer=Bank";
    const decode = vi.spyOn(qr, "decodeImage").mockResolvedValue(link);
    const update = vi.spyOn(api, "updateLogin").mockResolvedValue(BANK);
    renderPanel();

    await open("Bank");
    fireEvent.click(screen.getByRole("button", { name: "Edit Bank" }));
    const picture = new File(["png"], "qr.png", { type: "image/png" });
    fireEvent.change(screen.getByLabelText("QR code image"), { target: { files: [picture] } });
    expect(await screen.findByText("Read the key from the QR code.")).toBeInTheDocument();
    expect(decode).toHaveBeenCalledWith(picture);
    expect(screen.getByLabelText("Authenticator key")).toHaveAttribute("type", "password");
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(update).toHaveBeenCalledTimes(1));
    expect(update.mock.calls[0][1]).toMatchObject({ authenticator: link });
  });

  it("says when a picture holds no authenticator QR code", async () => {
    vi.spyOn(api, "logins").mockResolvedValue({ logins: [BANK] });
    vi.spyOn(api, "loginActivity").mockResolvedValue({ activity: [] });
    vi.spyOn(qr, "decodeImage").mockResolvedValue("https://bank.example/promo");
    renderPanel();

    await open("Bank");
    fireEvent.click(screen.getByRole("button", { name: "Edit Bank" }));
    fireEvent.change(screen.getByLabelText("QR code image"), {
      target: { files: [new File(["png"], "qr.png", { type: "image/png" })] },
    });
    expect(
      await screen.findByText("That QR code isn't an authenticator setup code."),
    ).toBeInTheDocument();
    expect(screen.getByLabelText("Authenticator key")).toHaveValue("");
  });

  it("lists a login's activity by day, with the conversation's title as its link", async () => {
    vi.spyOn(api, "logins").mockResolvedValue({ logins: [SHOP] });
    const activity = vi.spyOn(api, "loginActivity").mockResolvedValue({
      activity: [
        {
          at: new Date().toISOString(),
          used: "code_made",
          site: "https://shop.example",
          conversation: 12,
          conversation_title: "Buy black jeans",
        },
        {
          at: new Date(Date.now() - 2 * 24 * 60 * 60_000).toISOString(),
          used: "added",
          site: null,
          conversation: null,
        },
      ],
    });
    renderPanel();

    await open("Shop");
    const list = await screen.findByRole("list", { name: "Activity of Shop" });
    expect(activity).toHaveBeenCalledWith(1);
    expect(list).toHaveTextContent("Today");
    expect(list).toHaveTextContent("Code from the authenticator key entered · shop.example");
    expect(list).toHaveTextContent("Added");
    expect(screen.getByRole("link", { name: "Buy black jeans" })).toHaveAttribute(
      "href",
      "#/chat?conversation=12",
    );
  });

  it("offers Sign out only for a signed-in login", async () => {
    vi.spyOn(api, "logins").mockResolvedValue({ logins: [SHOP, BANK] });
    vi.spyOn(api, "loginActivity").mockResolvedValue({ activity: [] });
    const signOut = vi.spyOn(api, "signOutLogin").mockResolvedValue(undefined);
    renderPanel();

    await open("Shop");
    fireEvent.click(screen.getByRole("button", { name: "Sign out of Shop" }));
    await waitFor(() => expect(signOut).toHaveBeenCalledTimes(1));
    expect(signOut.mock.calls[0][0]).toBe(1);
    fireEvent.click(screen.getByRole("button", { name: "All logins" }));
    await open("Bank");
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

describe("byDay", () => {
  const use = (at: Date): LoginActivity => ({
    at: at.toISOString(),
    used: "signed_in",
    site: null,
    conversation: null,
  });

  it("groups uses as Today, Yesterday, then the date, newest first", () => {
    const now = new Date(2026, 10, 18, 15, 0);
    const groups = byDay(
      [
        use(new Date(2026, 10, 18, 9, 22)),
        use(new Date(2026, 10, 18, 8, 0)),
        use(new Date(2026, 10, 17, 23, 59)),
        use(new Date(2026, 10, 10, 12, 0)),
      ],
      now,
    );
    expect(groups.map((g) => g.day.startsWith("Today") || g.day.startsWith("Yesterday") ? g.day : "date")).toEqual([
      "Today",
      "Yesterday",
      "date",
    ]);
    expect(groups[0].uses).toHaveLength(2);
  });
});

describe("authenticatorFrom", () => {
  it("keeps an authenticator setup link and refuses anything else, saying why", () => {
    expect(qr.authenticatorFrom(" otpauth://totp/Shop:majse?secret=ABC ")).toEqual({
      link: "otpauth://totp/Shop:majse?secret=ABC",
    });
    expect(qr.authenticatorFrom("otpauth-migration://offline?data=xyz")).toHaveProperty(
      "refused",
    );
    expect(qr.authenticatorFrom("otpauth://hotp/Shop?secret=ABC&counter=1")).toHaveProperty(
      "refused",
    );
    expect(qr.authenticatorFrom("https://example.com")).toEqual({
      refused: "That QR code isn't an authenticator setup code.",
    });
  });
});
