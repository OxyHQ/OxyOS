// Stable module ids for the runtime's framework and the applications built against it
// (docs/adr/0030-installing-applications.mdx, decision 17). The framework (React Native's
// JavaScript, React, Bloom and the packages the host's native side implements) is compiled once
// per runtime; an application's bundle carries only its own modules and names the framework's by
// id, so an id is a contract between them, never Metro's numbering of one build.
//
// A module's id is 52 bits of the SHA-256 of its key:
// - a package's public module, `name@major:subpath|conditions`, the export (package.json's
//   `exports`) and the conditions Metro resolved it with (`react-native.import`) leading to the
//   file: one subpath may lead to two files, its ESM and its CommonJS builds; in a
//   package with no `exports`, every file is public, `name@major/path`. The major version (0.x's
//   minor) is in the key: a module stays the same across a package's compatible releases, and a
//   breaking one is another module;
// - a package's private module (a file no export names), `name@version/path`: the framework's own
//   business, which no application may name;
// - the runtime's own modules (`oxyos/`, the host's side of React Native and Expo), `oxyos/path`;
// - an application's own modules, `app/path` from the repository.
// Two keys with one id stop the build.
const crypto = require('crypto');
const fs = require('fs');
const path = require('path');

const PLATFORM_SUFFIX = /(\.(oxyos|android|native|ios))?\.([cm]?[jt]sx?|json)$/;

function majorOf(version) {
  const [major, minor] = version.split('.');
  return major === '0' ? `0.${minor}` : major;
}

// The nearest package.json above `file` that names a package, read once per directory.
const packages = new Map();
function packageOf(file) {
  let dir = path.dirname(file);
  while (dir !== path.dirname(dir)) {
    if (!packages.has(dir)) {
      const manifest = path.join(dir, 'package.json');
      let found = null;
      if (fs.existsSync(manifest)) {
        const json = JSON.parse(fs.readFileSync(manifest, 'utf8'));
        if (json.name && json.version) found = { dir, json, targets: exportTargets(dir, json) };
      }
      packages.set(dir, found);
    }
    const found = packages.get(dir);
    if (found) return found;
    dir = path.dirname(dir);
  }
  return null;
}

// The conditions Metro resolves with: `react-native` for the platform, and `import` or `require`
// by how a module is asked for (an `import` statement or `require()`), then `default`.
const CONDITIONS = ['react-native', 'import', 'require', 'default'];

// Every target an export's conditions lead to, with the conditions on the way (`react-native.import`),
// in the order Metro would consider them: one subpath may lead to two files (its ESM and its
// CommonJS build), each its own module.
function targets(value, conditions = []) {
  if (typeof value === 'string') return [{ target: value, conditions: conditions.join('.') }];
  if (value == null) return [];
  if (Array.isArray(value)) return value.flatMap((item) => targets(item, conditions));
  return CONDITIONS.filter((condition) => condition in value)
    .flatMap((condition) => targets(value[condition], [...conditions, condition]));
}

// Each export's targets, without their extensions: exact ones by file, patterns (`./icons/Ri*`) as
// [prefix, suffix] to match files against; the first export and conditions naming a file name it.
function exportTargets(dir, json) {
  if (!json.exports) return null;
  const exports = typeof json.exports === 'string' || !Object.keys(json.exports).some((k) => k.startsWith('.'))
    ? { '.': json.exports }
    : json.exports;
  const exact = new Map();
  const patterns = [];
  for (const [subpath, value] of Object.entries(exports)) {
    for (const { target, conditions } of targets(value)) {
      const absolute = path.join(dir, target).replace(PLATFORM_SUFFIX, '');
      if (subpath.includes('*')) {
        const [prefix, suffix] = absolute.split('*');
        patterns.push({ subpath, conditions, prefix, suffix: suffix ?? '' });
      } else if (!exact.has(absolute)) {
        exact.set(absolute, `${subpath}|${conditions}`);
      }
    }
  }
  return { exact, patterns };
}

function subpathOf(pkg, file) {
  const bare = file.replace(PLATFORM_SUFFIX, '');
  const exact = pkg.targets.exact.get(bare);
  if (exact) return exact;
  for (const { subpath, conditions, prefix, suffix } of pkg.targets.patterns) {
    if (bare.startsWith(prefix) && bare.endsWith(suffix) && bare.length >= prefix.length + suffix.length) {
      return `${subpath.replace('*', bare.slice(prefix.length, bare.length - suffix.length))}|${conditions}`;
    }
  }
  return null;
}

/**
 * A module's key, and whether it is public: `runtime` is the directory of the runtime's own
 * modules (`sample/oxyos`), `repository` the root an application's modules are named from.
 */
function keyOf(file, { runtime, repository }) {
  if (file.startsWith(runtime + path.sep)) return { key: `oxyos/${path.relative(runtime, file)}`, public: true };
  const pkg = packageOf(file);
  const inPackage = pkg && file.includes(`${path.sep}node_modules${path.sep}`) || (pkg && pkg.json.name === '@oxy.so/bloom');
  if (!inPackage) return { key: `app/${path.relative(repository, file)}`, public: true, app: true };
  const { name, version } = pkg.json;
  const relative = path.relative(pkg.dir, file);
  if (!pkg.targets) return { key: `${name}@${majorOf(version)}/${relative}`, public: true, name, version };
  const subpath = subpathOf(pkg, file);
  return subpath
    ? { key: `${name}@${majorOf(version)}:${subpath}`, public: true, name, version }
    : { key: `${name}@${version}/${relative}`, public: false, name, version };
}

function idOf(key) {
  return parseInt(crypto.createHash('sha256').update(key).digest('hex').slice(0, 13), 16);
}

/** Metro's `createModuleIdFactory`: each module's stable id, every key kept in `modules`. */
function createModuleIdFactory(options, modules) {
  return () => {
    const byId = new Map();
    return (file) => {
      const described = keyOf(file, options);
      const id = idOf(described.key);
      const other = byId.get(id);
      if (other && other.key !== described.key) {
        throw new Error(`module ids collide: ${other.key} and ${described.key}`);
      }
      // One key is one file: two copies of a package's module (two versions installed, resolved
      // by different requesters) would share an id, and the second would never run.
      if (other && other.file !== file) {
        throw new Error(`two copies of ${described.key}: ${other.file} and ${file}`);
      }
      byId.set(id, { key: described.key, file });
      modules.set(file, { ...described, id });
      return id;
    };
  };
}

/** Every module the builds of this process gave an id, by its file: what a framework's manifest lists. */
const recorded = new Map();

module.exports = { keyOf, idOf, majorOf, createModuleIdFactory, recorded };
