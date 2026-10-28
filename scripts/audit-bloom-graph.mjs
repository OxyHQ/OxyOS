#!/usr/bin/env node
// Reachable-dependency audit of the minimal native Bloom application (#33).
//
// Usage: node scripts/audit-bloom-graph.mjs <bloom-checkout> [--json]
//
// Resolves imports the way Metro does for a native target (`.native.*` before plain sources,
// the package's `react-native` export condition) starting from the Bloom entry points the
// minimal OxyOS application needs, and reports every third-party package reached, which Bloom
// files import it and how (static import, `require`, dynamic `import()`). It reads the Bloom
// checkout and writes nothing. It uses that checkout's own esbuild; it bundles nothing to disk.

import { createRequire } from 'node:module';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
import { readFileSync } from 'node:fs';

const [bloomArg, ...flags] = process.argv.slice(2);
if (!bloomArg) {
  console.error('usage: audit-bloom-graph.mjs <bloom-checkout> [--json]');
  process.exit(2);
}
const bloom = path.resolve(bloomArg);
const esbuild = createRequire(path.join(bloom, 'package.json'))('esbuild');
const revision = execFileSync('git', ['-C', bloom, 'rev-parse', 'HEAD'], { encoding: 'utf8' }).trim();
const pkg = createRequire(import.meta.url)(path.join(bloom, 'package.json'));

// The minimal application from the issue, plus the overlay/menu families #34 extends.
const ENTRIES = {
  BloomProvider: './provider',
  BloomScope: './appearance',
  AppShell: './app-shell',
  Screen: './screen',
  Surface: './surface',
  Portal: './portal',
  Dialog: './dialog',
  DropdownMenu: './dropdown-menu',
  Popover: './popover',
  Select: './select',
  Motion: './motion',
};

function entryFile(subpath) {
  const target = pkg.exports[subpath];
  if (!target) throw new Error(`no export ${subpath}`);
  const pick = (value) =>
    typeof value === 'string' ? value : pick(value['react-native'] ?? value.default ?? value.import);
  return path.join(bloom, pick(target));
}

const packageName = (specifier) =>
  specifier.startsWith('@') ? specifier.split('/').slice(0, 2).join('/') : specifier.split('/')[0];

async function audit(entry) {
  const reached = new Map();
  const record = {
    name: 'record-externals',
    setup(build) {
      build.onResolve({ filter: /^[^./]/ }, (args) => {
        if (args.path.startsWith('node:')) return { path: args.path, external: true };
        const name = packageName(args.path);
        const importer = path.relative(bloom, args.importer);
        const kinds = reached.get(name) ?? new Map();
        const kind = {
          'import-statement': 'static',
          'require-call': 'require',
          'dynamic-import': 'dynamic',
        }[args.kind] ?? args.kind;
        const set = kinds.get(kind) ?? new Set();
        set.add(importer);
        kinds.set(kind, set);
        reached.set(name, kinds);
        return { path: args.path, external: true };
      });
    },
  };
  const result = await esbuild.build({
    entryPoints: [entry],
    bundle: true,
    write: false,
    metafile: true,
    logLevel: 'silent',
    platform: 'neutral',
    format: 'esm',
    jsx: 'automatic',
    mainFields: ['react-native', 'module', 'main'],
    conditions: ['react-native'],
    resolveExtensions: ['.native.tsx', '.native.ts', '.tsx', '.ts', '.native.js', '.js', '.jsx', '.json'],
    loader: { '.ttf': 'empty', '.otf': 'empty', '.woff2': 'empty', '.png': 'empty', '.svg': 'empty', '.lottie': 'empty', '.riv': 'empty' },
    plugins: [record],
  });
  const files = Object.keys(result.metafile.inputs).filter((f) => !f.includes('node_modules'));
  // Named value imports from react-native: the core API surface the host must provide.
  const coreApi = new Set();
  for (const file of files) {
    const source = readFileSync(path.resolve(file), 'utf8');
    for (const match of source.matchAll(/import\s+(type\s+)?\{([^}]*)\}\s*from\s*['"]react-native['"]/g)) {
      if (match[1]) continue;
      for (const part of match[2].split(',')) {
        const name = part.trim();
        if (name && !name.startsWith('type ')) coreApi.add(name.split(/\s+as\s+/)[0]);
      }
    }
  }
  return { files: files.length, reached, coreApi };
}

const summary = new Map();
const perEntry = {};
const coreApi = new Set();
for (const [label, subpath] of Object.entries(ENTRIES)) {
  const { files, reached, coreApi: api } = await audit(entryFile(subpath));
  for (const name of api) coreApi.add(name);
  perEntry[label] = { files, packages: [...reached.keys()].sort() };
  for (const [name, kinds] of reached) {
    const entry = summary.get(name) ?? { entries: new Set(), kinds: new Map() };
    entry.entries.add(label);
    for (const [kind, importers] of kinds) {
      const set = entry.kinds.get(kind) ?? new Set();
      for (const importer of importers) set.add(importer);
      entry.kinds.set(kind, set);
    }
    summary.set(name, entry);
  }
}

const declared = (name) =>
  pkg.dependencies?.[name] ? `dependency ${pkg.dependencies[name]}`
    : pkg.peerDependencies?.[name]
      ? `peer ${pkg.peerDependencies[name]}${pkg.peerDependenciesMeta?.[name]?.optional ? ' (optional)' : ''}`
      : 'undeclared';

const rows = [...summary.entries()]
  .sort(([a], [b]) => a.localeCompare(b))
  .map(([name, { entries, kinds }]) => ({
    package: name,
    declared: declared(name),
    how: [...kinds.keys()].sort().join('+'),
    entries: [...entries].sort(),
    importers: Object.fromEntries([...kinds].map(([k, v]) => [k, [...v].sort()])),
  }));

if (flags.includes('--json')) {
  console.log(JSON.stringify({ bloom: revision, version: pkg.version, perEntry, packages: rows, reactNativeApi: [...coreApi].sort() }, null, 2));
} else {
  console.log(`Bloom ${pkg.version} @ ${revision}\n`);
  console.log('| Entry | Bloom source files | Third-party packages |');
  console.log('| --- | --- | --- |');
  for (const [label, { files, packages }] of Object.entries(perEntry)) {
    console.log(`| ${label} | ${files} | ${packages.map((p) => '`' + p + '`').join(', ')} |`);
  }
  console.log('\n| Package | Declared | How | Reached from |');
  console.log('| --- | --- | --- | --- |');
  for (const row of rows) {
    console.log(`| \`${row.package}\` | ${row.declared} | ${row.how} | ${row.entries.join(', ')} |`);
  }
  console.log(`\nreact-native value imports (${coreApi.size}): ${[...coreApi].sort().map((n) => '`' + n + '`').join(', ')}`);
}
