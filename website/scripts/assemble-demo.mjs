import {
  cpSync,
  existsSync,
  readFileSync,
  renameSync,
  rmSync,
} from 'node:fs';
import { resolve } from 'node:path';

const repositoryRoot = resolve(import.meta.dirname, '..');
const sourceDirectory = resolve(repositoryRoot, process.argv[2] ?? 'oxyos/shell/web-build');
const websiteDirectory = resolve(repositoryRoot, 'dist');
const demoDirectory = resolve(websiteDirectory, 'demo');
const sourceEntry = resolve(sourceDirectory, 'web.html');
const demoEntry = resolve(demoDirectory, 'index.html');

if (!existsSync(resolve(websiteDirectory, 'index.html'))) {
  throw new Error('Build the website before assembling the demo.');
}

if (!existsSync(sourceEntry)) {
  throw new Error(`OxyOS web entry is missing: ${sourceEntry}`);
}

rmSync(demoDirectory, { recursive: true, force: true });
cpSync(sourceDirectory, demoDirectory, { recursive: true });
renameSync(resolve(demoDirectory, 'web.html'), demoEntry);

const html = readFileSync(demoEntry, 'utf8');
if (!html.includes('/demo/assets/')) {
  throw new Error('The OxyOS web build is not configured for the /demo/ base path.');
}

console.log(`Assembled OxyOS demo at ${demoDirectory}`);
