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

// jsdom implements pointer events as plain MouseEvents and never
// defines the constructor, so Base UI's Switch — which synthesises a
// click on the hidden input — throws "PointerEvent is not a
// constructor" the moment a test toggles one. MouseEvent carries every
// field the components read; what is missing is only the name.
if (typeof window !== "undefined" && !window.PointerEvent) {
  window.PointerEvent = class PointerEvent extends MouseEvent {
    constructor(type: string, params: PointerEventInit = {}) {
      super(type, params);
    }
  } as unknown as typeof window.PointerEvent;
}
