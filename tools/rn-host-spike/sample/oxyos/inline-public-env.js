/**
 * Inlines `process.env.EXPO_PUBLIC_*` with its value when the bundle is built, everywhere,
 * node_modules included: what an Expo app's production build does (babel-preset-expo's
 * inline-env-vars, run by Expo's own transformer). This sample bundles with React Native's
 * transformer, whose Babel caller does not tell the preset it is a production build, so the preset
 * would leave a reference to `expo/virtual/env`, which throws in production bundles. A variable
 * that is not set becomes `undefined`, as Expo inlines it.
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
    if (!key || !key.startsWith('EXPO_PUBLIC_')) return;
    // Assignments are left alone.
    if (t.isAssignmentExpression(path.parent) && path.parent.left === node) return;
    path.replaceWith(t.valueToNode(process.env[key]));
  };
  return {
    name: 'oxyos-inline-public-env',
    visitor: { MemberExpression: visit, OptionalMemberExpression: visit },
  };
};
