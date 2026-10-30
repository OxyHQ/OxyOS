// Builds the development packages (docs/adr/0030-installing-applications.mdx, decision 1): each
// application's bundle on its own (entries/), built against the runtime's framework (framework.mjs,
// decision 17) so that it carries only its own modules, as a package of its description
// (packages/*.json) signed with a development key, and compiled on this machine with the runtime's
// compiler as oxyos-installd compiles a package; and the probes, together in one development
// bundle, against the framework too.
//
//   node packages.mjs OUT
//
// with BLOOM_DIR (the Bloom checkout), OXYOS_PACK (the oxyos-pack program) and OXYOS_RUNTIME (a
// runtime's directory: its runtime.json, hermesc and framework/). OUT gets one directory per package, named by
// its id, `applications/` (the Desktop Entry oxyos-installd writes for each, `oxyos-pack entry`),
// `probes/` (the probes' bundle, Hermes bytecode as bundle.mjs makes it with HERMESC) and
// `development.key`, made once and kept: a development key, never a publisher's.
import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';

const here = path.dirname(new URL(import.meta.url).pathname);
const [outDir] = process.argv.slice(2);
const pack = process.env.OXYOS_PACK;
const runtime = process.env.OXYOS_RUNTIME;
if (!outDir || !pack || !runtime || !process.env.BLOOM_DIR) {
  throw new Error('usage: BLOOM_DIR=... OXYOS_PACK=... OXYOS_RUNTIME=... node packages.mjs OUT');
}
const out = path.resolve(outDir);
fs.mkdirSync(out, { recursive: true });
const key = path.join(out, 'development.key');
const framework = path.join(runtime, 'framework', 'framework.json');
if (!fs.existsSync(framework)) throw new Error(`${runtime} has no framework: build it with framework.mjs`);
if (!fs.existsSync(key)) execFileSync(pack, ['key', key], { stdio: 'inherit' });

// Each application's icon, the artwork the registry already shows for it; the shell and the login
// screen, which the dock never shows, carry the OS's mark.
const repository = path.resolve(here, '../../..');
const icons = {
  'so.oxy.shell': 'config/includes.chroot_after_packages/usr/share/plymouth/themes/oxyos/mark-1200.png',
  'so.oxy.login': 'config/includes.chroot_after_packages/usr/share/plymouth/themes/oxyos/mark-1200.png',
  'so.oxy.settings': 'tools/shell-preview/assets/apps/settings.png',
  'so.oxy.inbox': 'tools/shell-preview/assets/apps/inbox.png',
  'so.oxy.allo': 'tools/shell-preview/assets/apps/allo.png',
  'so.oxy.stash': 'tools/shell-preview/assets/apps/stash.png',
};
const entries = {
  'so.oxy.shell': 'entries/shell.js',
  'so.oxy.login': 'entries/login.js',
  'so.oxy.settings': 'entries/settings.js',
  'so.oxy.inbox': 'entries/inbox.js',
  'so.oxy.allo': 'entries/allo.js',
  // Stash, from its own checkout (STASH_DIR, oxyos/application-build.js), when one is given.
  ...(process.env.STASH_DIR ? { 'so.oxy.stash': 'oxyos/stash-application.js' } : {}),
};
// STASH_DIR is Stash's build's alone: every other package, and the probes, are this sample's.
const { STASH_DIR: stash, ...environment } = process.env;

const run = (program, args, env = {}) =>
  execFileSync(program, args, { cwd: here, stdio: ['ignore', 'pipe', 'inherit'], env: { ...environment, ...env } })
    .toString()
    .trim();

for (const [id, entry] of Object.entries(entries)) {
  const dir = path.join(out, id);
  fs.rmSync(dir, { recursive: true, force: true });
  // The source bundle, never bytecode: HERMESC is not given to bundle.mjs.
  const bundled = JSON.parse(
    run('node', ['bundle.mjs', entry, path.join(dir, 'bundle')], {
      HERMESC: '',
      OXYOS_FRAMEWORK: framework,
      ...(id === 'so.oxy.stash' ? { STASH_DIR: stash } : {}),
    })
      .split('\n').pop());
  if (bundled.bytecode) throw new Error(`${id}: bundle.mjs wrote bytecode`);
  fs.copyFileSync(path.join(repository, icons[id]), path.join(dir, 'icon.png'));
  console.log(run(pack, ['pack', dir, path.join(here, 'packages', `${id}.json`), path.join(runtime, 'runtime.json'), key]));
  console.log(run(pack, ['compile', dir, path.join(runtime, 'hermesc')]));
  fs.mkdirSync(path.join(out, 'applications'), { recursive: true });
  fs.writeFileSync(path.join(out, 'applications', `${id}.desktop`), run(pack, ['entry', dir]) + '\n');
}

// The probes, which scenarios run by their modules: one development bundle, compiled as today.
const probes = path.join(out, 'probes');
fs.rmSync(probes, { recursive: true, force: true });
console.log(run('node', ['bundle.mjs', 'index.js', probes], { HERMESC: path.join(runtime, 'hermesc'), OXYOS_FRAMEWORK: framework }));
// The runtime they were compiled for, which runs them.
fs.copyFileSync(path.join(runtime, 'runtime.json'), path.join(probes, 'runtime.json'));
