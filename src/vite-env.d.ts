/// <reference types="vite/client" />

// TypeScript 6 errors (TS2882) on side-effect imports of modules that have no
// type declaration. Declare CSS as an untyped module so `import "./x.css"`
// (loaded by Vite at build time) type-checks.
declare module "*.css";
