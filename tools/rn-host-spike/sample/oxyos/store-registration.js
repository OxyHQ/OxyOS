/**
 * The machine's installer, registered in the application's own Oxy SDK before its code runs, as
 * @oxy.so/core asks of a runtime that installs applications (`setStoreInstaller`; ADR 0030,
 * decision 9; oxy ADR 0053). Only a launch the image's policy gives `store` has one
 * (oxyos/store-installer.js); every other application's `getStoreInstaller()` stays null.
 *
 * Part of the application's bundle, not the framework: `@oxy.so/core` is the application's, and
 * this resolves it as the application's root does (metro.config.js), so it is the very copy the
 * application reads. An application whose SDK has no such seam (older than 7.10) builds this as
 * oxyos/no-store-registration.js, which registers nothing (oxyos/application-build.js).
 */
import { setStoreInstaller, StoreInstallerError } from '@oxy.so/core';
import { storeInstaller } from './store-installer';

const installer = storeInstaller(StoreInstallerError);
if (installer) setStoreInstaller(installer);
