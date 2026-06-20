// Global typing for the dev test bus so specs (outside the app tsconfig) compile.
import type { KodenTestBus } from "../../src/dev/testBus";

declare global {
  interface Window {
    __KODEN_TEST__: KodenTestBus;
  }
}

export {};
