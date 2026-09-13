import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import { SignIn } from "@/components/SignIn";

describe("SignIn", () => {
  it("sends the browser to GitHub rather than fetching", () => {
    // The binding cookie has to be set on a response the browser
    // actually follows, so this must be a navigation.
    render(<SignIn />);
    const link = screen.getByRole("link", { name: /Sign in with GitHub/ });
    expect(link).toHaveAttribute("href", expect.stringContaining("/auth/github/start"));
  });

  it("brings you back where you were", () => {
    render(<SignIn />);
    const href = screen.getByRole("link", { name: /Sign in with GitHub/ }).getAttribute("href");
    expect(href).toContain("next=");
  });

  it("says why the last attempt was refused", () => {
    // The usual cause is an address Niles was never told about, and the
    // fix is to add it — which needs the message to survive the redirect.
    render(<SignIn error="nobody here uses someone@example.com" />);
    expect(screen.getByText(/nobody here uses/)).toBeInTheDocument();
  });

  it("says a GitHub account alone is not enough", () => {
    render(<SignIn />);
    expect(screen.getByText(/not enough/)).toBeInTheDocument();
  });
});
