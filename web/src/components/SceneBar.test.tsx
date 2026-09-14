import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { SceneBar } from "@/components/SceneBar";

describe("SceneBar", () => {
  it("shows nothing at all when none are saved", () => {
    // An empty shelf labelled Scenes teaches somebody the feature is
    // missing, when what is missing is that they have not saved one.
    const { container } = render(<SceneBar scenes={[]} onApply={vi.fn()} />);
    expect(container).toBeEmptyDOMElement();
  });

  it("offers one press per scene", () => {
    const onApply = vi.fn();
    render(<SceneBar scenes={["cosy", "movie_night"]} onApply={onApply} />);
    fireEvent.click(screen.getByRole("button", { name: /Cosy/ }));
    expect(onApply).toHaveBeenCalledWith("cosy");
  });

  it("applies by the name it was saved under, not the one shown", () => {
    // The button reads "Movie night"; the store knows it as
    // `movie_night`, and sending the pretty one would find nothing.
    const onApply = vi.fn();
    render(<SceneBar scenes={["movie_night"]} onApply={onApply} />);
    expect(screen.getByText("Movie night")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /Movie night/ }));
    expect(onApply).toHaveBeenCalledWith("movie_night");
  });
});
