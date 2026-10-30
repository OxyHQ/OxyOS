// Metro's asset plugin for the runtime's framework build (docs/adr/0030-installing-applications.mdx,
// decision 17): marks each asset the framework registers as the framework's, so it is resolved
// beside the framework rather than beside the application that shows it (framework-init.js).
module.exports = (asset) => ({ ...asset, oxyosFramework: true });
