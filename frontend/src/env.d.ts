/// <reference types="vite/client" />

declare module '*.vue' {
  import type { DefineComponent } from 'vue';
  const component: DefineComponent<object, object, unknown>;
  export default component;
}

/** Injected by vite `define` from package.json (keep in sync with Cargo.toml). */
declare const __PLUGIN_VERSION__: string;

interface Window {
  $dialog?: {
    warning: (options: Record<string, unknown>) => void;
    success: (options: Record<string, unknown>) => void;
    error: (options: Record<string, unknown>) => void;
    info: (options: Record<string, unknown>) => void;
  };
  $message?: {
    success: (text: string) => void;
    error: (text: string) => void;
    info: (text: string) => void;
    warning: (text: string) => void;
  };
}
