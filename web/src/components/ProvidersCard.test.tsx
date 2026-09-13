import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { ProvidersCard } from "@/components/ProvidersCard";
import type { Provider } from "@/lib/api";

const GROQ: Provider = {
  name: "groq",
  base_url: "https://api.groq.com/openai/v1",
};

function add(name: string, url: string) {
  fireEvent.change(screen.getByLabelText("Provider name"), {
    target: { value: name },
  });
  fireEvent.change(screen.getByLabelText("Provider endpoint"), {
    target: { value: url },
  });
  fireEvent.click(screen.getByRole("button", { name: /Add/ }));
}

describe("ProvidersCard", () => {
  it("adds one", () => {
    const onChange = vi.fn();
    render(<ProvidersCard providers={[]} onChange={onChange} />);
    add("groq", "https://api.groq.com/openai/v1");
    expect(onChange).toHaveBeenCalledWith([GROQ]);
  });

  it("lower-cases the name so a role can refer to it", () => {
    // `provider = "Groq"` and `provider = "groq"` would otherwise be
    // two providers with one key between them.
    const onChange = vi.fn();
    render(<ProvidersCard providers={[]} onChange={onChange} />);
    add("Groq", "https://api.groq.com/openai/v1");
    expect(onChange).toHaveBeenCalledWith([GROQ]);
  });

  it("refuses a second one with the same name", () => {
    const onChange = vi.fn();
    render(<ProvidersCard providers={[GROQ]} onChange={onChange} />);
    add("groq", "https://elsewhere.test/v1");
    expect(onChange).not.toHaveBeenCalled();
    expect(screen.getByText(/already a provider called groq/)).toBeInTheDocument();
  });

  it("refuses an endpoint that is not a URL", () => {
    // The server checks too. Saying it here means the answer arrives
    // while the thing being described is still on screen.
    const onChange = vi.fn();
    render(<ProvidersCard providers={[]} onChange={onChange} />);
    add("groq", "api.groq.com");
    expect(onChange).not.toHaveBeenCalled();
    expect(screen.getByText(/has to be a URL/)).toBeInTheDocument();
  });

  it("removes one", () => {
    const onChange = vi.fn();
    render(<ProvidersCard providers={[GROQ]} onChange={onChange} />);
    fireEvent.click(screen.getByRole("button", { name: "Remove groq" }));
    expect(onChange).toHaveBeenCalledWith([]);
  });

  it("says what happens with none, rather than looking broken", () => {
    render(<ProvidersCard providers={[]} onChange={vi.fn()} />);
    expect(screen.getByText(/fall back to whatever the config/)).toBeInTheDocument();
  });
});
