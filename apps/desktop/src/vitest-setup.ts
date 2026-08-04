import "@testing-library/jest-dom/vitest";

// jsdom doesn't implement scrollTo — a known gap, not a real bug in
// whatever component calls it (e.g. ChatPane's scroll-to-bottom effect).
Element.prototype.scrollTo = () => {};
