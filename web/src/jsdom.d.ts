// jsdom is what the tests run in, and a test of the sealed frame's
// runtime runs it in a window of its own. The package ships no types
// and @types/jsdom would be a dependency for one test; this is the
// little of it that test uses.
declare module "jsdom" {
  export interface ConstructorOptions {
    runScripts?: "dangerously" | "outside-only";
    url?: string;
    beforeParse?: (window: Window & typeof globalThis) => void;
  }
  export class JSDOM {
    constructor(html?: string, options?: ConstructorOptions);
    readonly window: Window & typeof globalThis;
  }
}
