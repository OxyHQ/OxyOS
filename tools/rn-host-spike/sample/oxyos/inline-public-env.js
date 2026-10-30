/**
 * Inlines `process.env.EXPO_PUBLIC_*` with its value when the bundle is built, everywhere,
 * node_modules included: what an Expo app's production build does (babel-preset-expo's
 * inline-env-vars, run by Expo's own transformer). This sample bundles with React Native's
 * transformer, whose Babel caller does not tell the preset it is a production build, so the preset
 * would leave a reference to `expo/virtual/env`, which throws in production bundles. A variable
 * that is not set becomes `undefined`, as Expo inlines it.
 *
 * `process.env.OXY_BUILD_ID` too, the build the Oxy SDK busts its persisted caches on, as an Oxy
 * application's own production build inlines it (@oxy.so/app-preset's Babel config): the
 * application's commit (oxyos/application-build.js), else `undefined`.
 */
module.exports = function inlinePublicEnv({ types: t }) {
  const isProcessEnv = (node) =>
    t.isMemberExpression(node) &&
    t.isIdentifier(node.object, { name: 'process' }) &&
    t.isIdentifier(node.property, { name: 'env' });
  const visit = (path) => {
    const { node } = path;
    if (!isProcessEnv(node.object)) return;
    const key = t.isIdentifier(node.property) && !node.computed ? node.property.name
      : t.isStringLiteral(node.property) ? node.property.value : null;
    if (!key || !(key.startsWith('EXPO_PUBLIC_') || key === 'OXY_BUILD_ID')) return;
    // Assignments are left alone.
    if (t.isAssignmentExpression(path.parent) && path.parent.left === node) return;
    path.replaceWith(t.valueToNode(process.env[key]));
  };
  return {
    name: 'oxyos-inline-public-env',
    visitor: { MemberExpression: visit, OptionalMemberExpression: visit },
  };
};
