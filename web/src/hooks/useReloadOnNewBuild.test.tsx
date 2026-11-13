import { beforeEach, describe, expect, it, vi } from "vitest";
import { renderHook } from "@testing-library/react";
import { forgetBuild } from "@/lib/update";
import { useReloadOnNewBuild } from "@/hooks/useReloadOnNewBuild";

/** The acceptance test for "reloads itself once and only once": across
    a redeploy the reload fires exactly one time. */
describe("useReloadOnNewBuild", () => {
  beforeEach(() => forgetBuild());

  it("reloads once when the answering build changes", () => {
    const reload = vi.fn();
    const { rerender } = renderHook(
      ({ commit }) => useReloadOnNewBuild(commit, reload),
      { initialProps: { commit: undefined as string | null | undefined } },
    );
    rerender({ commit: "a" });
    rerender({ commit: "a" });
    rerender({ commit: "b" });
    rerender({ commit: "b" });
    rerender({ commit: "c" });
    expect(reload).toHaveBeenCalledTimes(1);
  });

  it("ignores the no-answer-yet state and adopts a dev build's null", () => {
    const reload = vi.fn();
    const { rerender } = renderHook(
      ({ commit }) => useReloadOnNewBuild(commit, reload),
      { initialProps: { commit: undefined as string | null | undefined } },
    );
    rerender({ commit: null });
    rerender({ commit: null });
    expect(reload).not.toHaveBeenCalled();
    rerender({ commit: "a" });
    expect(reload).toHaveBeenCalledTimes(1);
  });
});
