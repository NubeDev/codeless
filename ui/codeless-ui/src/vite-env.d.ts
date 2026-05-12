/// <reference types="vite/client" />

interface ImportMetaEnv {
  readonly VITE_CODELESS_BASE_URL?: string;
  readonly VITE_CODELESS_TOKEN?: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}
