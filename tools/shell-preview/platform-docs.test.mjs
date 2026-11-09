// docs/platform-api.mdx is what @oxyos/platform's types say (docs/adr/0058-the-platform-api.mdx,
// decision 9): regenerate it with `node platform-docs.mjs` when they change.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import { test } from 'node:test';

import { render } from './platform-docs.mjs';

test("the platform API's page is generated from the package and has not drifted", () => {
  const written = fs.readFileSync(new URL('../../docs/platform-api.mdx', import.meta.url), 'utf8');
  assert.equal(written, render(), 'docs/platform-api.mdx differs from @oxyos/platform: run `node platform-docs.mjs`');
});
