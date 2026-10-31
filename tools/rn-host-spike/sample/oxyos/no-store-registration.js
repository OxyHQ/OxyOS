// What oxyos/store-registration.js is built as for an application whose Oxy SDK has no store
// installer seam (older than @oxy.so/core 7.10, oxyos/application-build.js): nothing is registered,
// and the application's `getStoreInstaller()`, if it has one, stays null. A module of the
// application's own, so its bundle needs nothing of the framework for it.
export {};
