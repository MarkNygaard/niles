import { afterEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { Outcome, startWaiting, stopWaiting } from "@/lib/restart";
import { RestartingScreen } from "@/components/RestartingScreen";

vi.mock("@/hooks/useDeviceStream", () => ({ useDeviceStream: () => {} }));

describe("RestartingScreen", () => {
  afterEach(() => stopWaiting());

  it("shows nothing when nobody is waiting", () => {
    const { container } = render(<RestartingScreen wait={() => Promise.resolve("back" as Outcome)} />);
    expect(container).toBeEmptyDOMElement();
  });

  it("waits, and reloads when the new Niles answers", async () => {
    startWaiting({ before: "boot-a", supervisor: "kubernetes" });
    const onBack = vi.fn();
    render(
      <RestartingScreen onBack={onBack} wait={() => Promise.resolve("back" as Outcome)} />,
    );
    expect(await screen.findByText("Restarting…")).toBeInTheDocument();
    await waitFor(() => expect(onBack).toHaveBeenCalledTimes(1));
  });

  it("gives up with the log hint, and keeps waiting on ask", async () => {
    startWaiting({ before: "boot-a", supervisor: "kubernetes" });
    let waits = 0;
    const wait = vi.fn((_: string) => {
      waits += 1;
      return Promise.resolve(waits === 1 ? ("gave_up" as Outcome) : ("back" as Outcome));
    });
    const onBack = vi.fn();
    render(<RestartingScreen onBack={onBack} wait={wait} />);
    expect(
      await screen.findByText("Niles hasn't come back after two minutes."),
    ).toBeInTheDocument();
    expect(screen.getByText("kubectl logs deploy/niles")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Keep waiting" }));
    await waitFor(() => expect(waits).toBe(2));
    await waitFor(() => expect(onBack).toHaveBeenCalledTimes(1));
  });

  it("reloads from the gave-up state on Reload", async () => {
    startWaiting({ before: "boot-a", supervisor: null });
    const onBack = vi.fn();
    render(<RestartingScreen onBack={onBack} wait={() => Promise.resolve("gave_up" as Outcome)} />);
    fireEvent.click(await screen.findByRole("button", { name: "Reload" }));
    expect(onBack).toHaveBeenCalledTimes(1);
  });
});
