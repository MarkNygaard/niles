import { afterEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { ChatGptPanel } from "@/components/ChatGptPanel";
import type { ChatGptPanelProps } from "@/components/ChatGptPanel";
import { api } from "@/lib/api";
import type { ChatGptStatus } from "@/lib/api";

const SIGNED_OUT: ChatGptStatus = {
  signed_in: false,
  email: null,
  expires_at: null,
  writable: true,
};

function setup(props: Partial<ChatGptPanelProps> = {}) {
  const onEnabled = vi.fn();
  const onModel = vi.fn();
  const onChanged = vi.fn();
  render(
    <ChatGptPanel
      enabled
      model="gpt-5.6-terra"
      status={SIGNED_OUT}
      onEnabled={onEnabled}
      onModel={onModel}
      onChanged={onChanged}
      {...props}
    />,
  );
  return { onEnabled, onModel, onChanged };
}

afterEach(() => {
  vi.restoreAllMocks();
});

describe("ChatGptPanel", () => {
  it("says who is signed in, and offers to sign out", async () => {
    const signOut = vi.spyOn(api, "chatgptSignOut").mockResolvedValue(undefined);
    const { onChanged } = setup({
      status: { ...SIGNED_OUT, signed_in: true, email: "bro@example.com" },
    });
    expect(screen.getByText("bro@example.com")).toBeInTheDocument();
    expect(screen.getByText(/Signed in as/)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Sign in with ChatGPT/ })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Sign out" }));
    await waitFor(() => expect(onChanged).toHaveBeenCalled());
    expect(signOut).toHaveBeenCalled();
  });

  it("says before it happens that the page after signing in will fail to load", () => {
    setup();
    expect(screen.getByText("Not signed in")).toBeInTheDocument();
    expect(screen.getByText(/the page that opens will fail to load/)).toBeInTheDocument();
    expect(screen.getByText(/copy its address/)).toBeInTheDocument();
    expect(
      screen.getByRole("textbox", { name: "Address of the page that failed to load" }),
    ).toBeInTheDocument();
  });

  it("opens the sign-in page in a new tab", async () => {
    const tab = { opener: {}, location: { href: "" }, close: vi.fn() };
    const open = vi.spyOn(window, "open").mockReturnValue(tab as unknown as Window);
    vi.spyOn(api, "chatgptSignIn").mockResolvedValue({
      authorize_url: "https://auth.openai.com/oauth/authorize?state=s",
    });
    setup();
    fireEvent.click(screen.getByRole("button", { name: /Sign in with ChatGPT/ }));
    expect(open).toHaveBeenCalledWith("about:blank", "_blank");
    await waitFor(() =>
      expect(tab.location.href).toBe("https://auth.openai.com/oauth/authorize?state=s"),
    );
    expect(tab.opener).toBeNull();
    // And as a link, for a browser that would not open the tab.
    expect(screen.getByRole("link", { name: /Open the sign-in page/ })).toHaveAttribute(
      "href",
      "https://auth.openai.com/oauth/authorize?state=s",
    );
  });

  it("connects with what was pasted", async () => {
    const finish = vi.spyOn(api, "chatgptFinish").mockResolvedValue({ email: "bro@example.com" });
    const { onChanged } = setup();
    const connect = screen.getByRole("button", { name: "Connect" });
    expect(connect).toBeDisabled();
    fireEvent.change(
      screen.getByRole("textbox", { name: "Address of the page that failed to load" }),
      { target: { value: "  http://localhost:1455/auth/callback?code=c&state=s " } },
    );
    fireEvent.click(connect);
    await waitFor(() => expect(onChanged).toHaveBeenCalled());
    expect(finish).toHaveBeenCalledWith("http://localhost:1455/auth/callback?code=c&state=s");
  });

  it("says why a paste did not connect", async () => {
    vi.spyOn(api, "chatgptFinish").mockRejectedValue(
      new Error("That sign-in has expired, or was not started here."),
    );
    const { onChanged } = setup();
    fireEvent.change(
      screen.getByRole("textbox", { name: "Address of the page that failed to load" }),
      { target: { value: "code" } },
    );
    fireEvent.click(screen.getByRole("button", { name: "Connect" }));
    expect(await screen.findByText(/has expired/)).toBeInTheDocument();
    expect(onChanged).not.toHaveBeenCalled();
  });

  it("says a sign-in cannot be kept without a database", () => {
    setup({ status: { ...SIGNED_OUT, writable: false } });
    expect(screen.getByText(/needs Niles's database/)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Sign in with ChatGPT/ })).toBeNull();
  });

  it("switches off and picks a model", () => {
    const { onEnabled, onModel } = setup();
    fireEvent.click(screen.getByRole("switch"));
    expect(onEnabled).toHaveBeenCalledWith(false);
    fireEvent.click(screen.getByRole("radio", { name: /GPT-5.6 Luna/ }));
    expect(onModel).toHaveBeenCalledWith("gpt-5.6-luna");
  });
});
