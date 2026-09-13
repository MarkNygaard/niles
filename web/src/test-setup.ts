import "@testing-library/jest-dom/vitest";

// jsdom has no matchMedia at all, and a component that asks the
// viewport a question should get an answer rather than a TypeError.
// The default answer is "no": tests that care which way it goes stub
// this themselves.
if (typeof window !== "undefined" && !window.matchMedia) {
  window.matchMedia = (query: string) =>
    ({
      matches: false,
      media: query,
      onchange: null,
      addEventListener: () => {},
      removeEventListener: () => {},
      addListener: () => {},
      removeListener: () => {},
      dispatchEvent: () => false,
    }) as MediaQueryList;
}
