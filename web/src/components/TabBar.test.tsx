import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import { TabBar } from "@/components/TabBar";

describe("TabBar", () => {
  it("marks the house as where you are on the front page", () => {
    render(<TabBar route="/" />);
    expect(screen.getByRole("link", { name: "Home" })).toHaveAttribute("aria-current", "page");
    expect(screen.getByRole("link", { name: "Groceries" })).not.toHaveAttribute("aria-current");
  });

  it("marks the list on the list", () => {
    render(<TabBar route="/groceries" />);
    expect(screen.getByRole("link", { name: "Groceries" })).toHaveAttribute(
      "aria-current",
      "page",
    );
  });

  it("marks Settings when Settings was put here", () => {
    render(<TabBar route="/me/settings" menu={[{ id: "settings", hidden: false }]} />);
    expect(screen.getByRole("link", { name: "Settings" })).toHaveAttribute("aria-current", "page");
  });

  it("has no Me: that is the avatar's", () => {
    render(<TabBar route="/" />);
    expect(screen.queryByRole("link", { name: "Me" })).toBeNull();
  });

  it("puts the entries in the arranged order and leaves out the hidden", () => {
    render(
      <TabBar
        route="/"
        menu={[
          { id: "chat", hidden: false },
          { id: "groceries", hidden: true },
        ]}
      />,
    );
    const labels = screen.getAllByRole("link").map((a) => a.textContent);
    expect(labels).toEqual(["Home", "Chat"]);
  });
});

describe("TabBar with a plugin's page", () => {
  const pages = [
    {
      id: "spotprice.electricity",
      plugin: "spotprice",
      page: "electricity",
      label: "Electricity",
      icon: "zap",
      refresh_every: null,
    },
  ];

  it("links to the page's route and marks it while it is open", () => {
    render(
      <TabBar
        route="/plugins/spotprice/electricity"
        menu={[{ id: "spotprice.electricity", hidden: false }]}
        pages={pages}
      />,
    );
    const link = screen.getByRole("link", { name: "Electricity" });
    expect(link).toHaveAttribute("href", "#/plugins/spotprice/electricity");
    expect(link).toHaveAttribute("aria-current", "page");
  });

  it("leaves out an entry whose plugin is gone", () => {
    render(<TabBar route="/" menu={[{ id: "gone.page", hidden: false }]} pages={pages} />);
    expect(screen.getAllByRole("link").map((a) => a.textContent)).toEqual(["Home"]);
  });
});
