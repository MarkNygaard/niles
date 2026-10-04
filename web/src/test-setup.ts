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

// nwsapi 2.2.27, jsdom's selector engine, answers `:modal` by asking
// `:fullscreen`, and answers that by calling `matches(":fullscreen")`
// on the same element — itself — until the stack overflows. floating-ui
// asks `:modal` on every position it computes for a popover and catches
// the overflow, so each popover cost seconds of CPU that went on after
// its test ended: three in one file starved the worker into "Timeout
// calling onTaskUpdate" on CI. jsdom has no top layer and no
// fullscreen, so for exactly these two the answer is no.
if (typeof Element !== "undefined") {
  const matches = Element.prototype.matches;
  const never = new Set([":modal", ":fullscreen"]);
  Element.prototype.matches = function (this: Element, selector: string) {
    return never.has(selector) ? false : matches.call(this, selector);
  };
}
