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
// runtime's directory: its runtime.json, hermesc and framework/), and, for each application built
// from its own checkout (`CHECKOUTS` below: DROPSEA_DIR, ATLAS_DIR), that checkout's Expo project. OUT gets one directory per package, named by
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

// Each application's icon, the artwork the registry already shows for it; the shell, the login
// screen and About This Computer, which the dock never shows, carry the OS's mark.
const repository = path.resolve(here, '../../..');
const icons = {
  'so.oxy.shell': 'config/includes.chroot_after_packages/usr/share/plymouth/themes/oxyos/mark-1200.png',
  'so.oxy.login': 'config/includes.chroot_after_packages/usr/share/plymouth/themes/oxyos/mark-1200.png',
  'so.oxy.settings': 'tools/shell-preview/assets/apps/settings.png',
  'so.oxy.aboutthiscomputer': 'config/includes.chroot_after_packages/usr/share/plymouth/themes/oxyos/mark-1200.png',
  'so.oxy.activitymonitor': 'tools/shell-preview/assets/apps/activity-monitor.png',
  'so.oxy.inbox': 'tools/shell-preview/assets/apps/inbox.png',
  'so.oxy.allo': 'tools/shell-preview/assets/apps/allo.png',
  'so.oxy.dropsea': 'tools/shell-preview/assets/apps/dropsea.png',
  'so.oxy.atlas': 'tools/shell-preview/assets/apps/atlas.png',
  'so.oxy.permissionprobe': 'tools/shell-preview/assets/apps/inbox.png',
  'so.oxy.backgroundprobe': 'tools/shell-preview/assets/apps/allo.png',
  'so.oxy.crashprobe': 'tools/shell-preview/assets/apps/inbox.png',
  'so.oxy.hangprobe': 'tools/shell-preview/assets/apps/allo.png',
};
// Each application's declared searchable items (docs/adr/0051-search.mdx): `searchable.json` at its
// package's root, beside `manifest.json` and `index.hbc`, listed in the manifest as every file is.
// The search service reads `<package>/searchable.json` of each application the registry lists, as a
// Mac's preference panes declare their search terms: Settings' pages, each with its title and the
// words a person would type to find it, in each language (apps/settings/searchable.json).
const searchable = {
  'so.oxy.settings': 'apps/settings/searchable.json',
};
const entries = {
  'so.oxy.shell': 'entries/shell.js',
  'so.oxy.login': 'entries/login.js',
  'so.oxy.settings': 'entries/settings.js',
  'so.oxy.aboutthiscomputer': 'entries/about-this-computer.js',
  'so.oxy.activitymonitor': 'entries/activity-monitor.js',
  'so.oxy.inbox': 'entries/inbox.js',
  'so.oxy.allo': 'entries/allo.js',
  // A probe installed as an application of its own (the scenarios' install service publishes it).
  'so.oxy.permissionprobe': 'entries/permission.js',
  // A probe with a login item and background activity
  // (docs/adr/0060-login-items-and-background-activity.mdx).
  'so.oxy.backgroundprobe': 'entries/background.js',
  // An application that quits unexpectedly while its data is corrupt (docs/adr/0061), installed by
  // the crash scenarios.
  'so.oxy.crashprobe': 'entries/crash.js',
  // An application that hangs (docs/adr/0067), installed by the hang scenarios.
  'so.oxy.hangprobe': 'entries/hang.js',
};
// The applications built from their own checkouts (docs/react-native-android-adaptation.mdx), each
// named by its variable and built when it is given: the runtime's entry (oxyos/application.js)
// over the checkout's Expo Router root, registered as its package's module. Dropsea and Atlas are
// system applications (docs/adr/0030-installing-applications.mdx, decision 13).
const CHECKOUTS = { 'so.oxy.dropsea': 'DROPSEA_DIR', 'so.oxy.atlas': 'ATLAS_DIR' };
const checkouts = Object.fromEntries(
  Object.entries(CHECKOUTS)
    .filter(([, variable]) => process.env[variable])
    .map(([id, variable]) => [id, path.resolve(process.env[variable])]),
);
for (const id of Object.keys(checkouts)) entries[id] = 'oxyos/application.js';
// A checkout's variable is its own build's alone: every other package, and the probes, are this
// sample's.
const environment = Object.fromEntries(
  Object.entries(process.env).filter(([name]) => !Object.values(CHECKOUTS).includes(name)),
);
const description = (id) => JSON.parse(fs.readFileSync(path.join(here, 'packages', `${id}.json`), 'utf8'));

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
      ...(checkouts[id]
        ? { OXYOS_APPLICATION_DIR: checkouts[id], OXYOS_APPLICATION_MODULE: description(id).entry.module }
        : {}),
    })
      .split('\n').pop());
  if (bundled.bytecode) throw new Error(`${id}: bundle.mjs wrote bytecode`);
  fs.copyFileSync(path.join(repository, icons[id]), path.join(dir, 'icon.png'));
  if (searchable[id]) fs.copyFileSync(path.join(repository, searchable[id]), path.join(dir, 'searchable.json'));
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
