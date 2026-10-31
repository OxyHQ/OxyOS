// The platform adapter the fidelity gallery runs with on OxyOS: the one an application gets there
// (popups as compositor popups). The web build takes FidelityPlatform.web.ts instead.
export { oxyosSurfaces as fidelityPlatform } from './oxyos/platform-surface';
