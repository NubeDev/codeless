import "@testing-library/jest-dom/vitest";

// jsdom lacks Element.scrollIntoView; UI components (StageChat, etc.)
// call it inside effects and the bare TypeError crashes React's commit
// phase, taking the whole render with it. Stubbing once on the
// prototype keeps every test honest without per-test setup noise.
if (!Element.prototype.scrollIntoView) {
  Object.defineProperty(Element.prototype, "scrollIntoView", {
    value: () => undefined,
    writable: true,
  });
}

// Same story for ResizeObserver and matchMedia — present in real
// browsers, absent in jsdom, occasionally hit by motion/ScrollArea.
if (typeof window !== "undefined") {
  if (typeof window.ResizeObserver === "undefined") {
    window.ResizeObserver = class {
      observe() {}
      unobserve() {}
      disconnect() {}
    } as unknown as typeof ResizeObserver;
  }
  if (typeof window.matchMedia === "undefined") {
    window.matchMedia = (query: string) => ({
      matches: false,
      media: query,
      onchange: null,
      addListener: () => {},
      removeListener: () => {},
      addEventListener: () => {},
      removeEventListener: () => {},
      dispatchEvent: () => false,
    }) as MediaQueryList;
  }
}
