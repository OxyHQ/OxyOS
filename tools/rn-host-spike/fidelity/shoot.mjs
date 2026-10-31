// Chromium's frames of the fidelity gallery's web export (fidelity/web.mjs), for fidelity.py: each
// page in each configuration at exactly the window's logical size, at the configuration's device
// scale factor and color scheme, motion reduced, CSS animations and transitions finished, no scroll
// bars; once the gallery says every cell is laid out and its fonts are loaded, and two frames a
// beat apart are the same, the page is shot whole and the gallery's layout written beside it:
//
//   node fidelity/shoot.mjs '{"dist": DIR, "out": DIR, "pages": [{page, width, height}],
//                             "configs": [{scheme, scale, name}]}'
//
// writes OUT/page<N>-<name>.png and .json ({cells: {id: {x, y, w, h, cw, ch, label}}, dom: ...}).
import fs from 'node:fs';
import http from 'node:http';
import path from 'node:path';
import { chromium } from 'playwright-core';

const spec = JSON.parse(process.argv[2]);
const types = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.ttf': 'font/ttf',
  '.woff2': 'font/woff2', '.png': 'image/png', '.json': 'application/json', '.svg': 'image/svg+xml' };
const requested = new Set();
const server = http.createServer((request, response) => {
  const url = new URL(request.url, 'http://localhost');
  let file = path.join(spec.dist, decodeURIComponent(url.pathname));
  if (!file.startsWith(spec.dist)) return response.writeHead(403).end();
  if (fs.existsSync(file) && fs.statSync(file).isDirectory()) file = path.join(file, 'index.html');
  if (!fs.existsSync(file)) return response.writeHead(404).end();
  requested.add(path.relative(spec.dist, file));
  response.writeHead(200, { 'content-type': types[path.extname(file)] ?? 'application/octet-stream' });
  fs.createReadStream(file).pipe(response);
});
await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
const origin = `http://127.0.0.1:${server.address().port}`;

// No animation runs at the shot: whatever is left of a CSS one ends at once, and no caret blinks.
const STILL = `*, *::before, *::after { animation-duration: 0s !important; animation-delay: 0s !important;
  transition-duration: 0s !important; transition-delay: 0s !important; caret-color: transparent !important; }`;

const browser = await chromium.launch({ headless: true });
let failed = false;
try {
  for (const config of spec.configs) {
    for (const page of spec.pages) {
      const name = `page${page.page}-${config.name}`;
      const context = await browser.newContext({
        viewport: { width: page.width, height: page.height },
        deviceScaleFactor: config.scale,
        colorScheme: config.scheme,
        reducedMotion: 'reduce',
      });
      const tab = await context.newPage();
      const errors = [];
      tab.on('pageerror', (error) => errors.push(String(error)));
      tab.on('console', (message) => { if (message.type() === 'error') errors.push(message.text()); });
      await tab.goto(`${origin}/?page=${page.page}`);
      await tab.addStyleTag({ content: STILL });
      try {
        await tab.waitForFunction(() => window.__fidelity?.ready === true, null, { timeout: 30000 });
      } catch {
        console.error(`${name}: the gallery never said it was laid out; errors: ${errors.join(' | ')}`);
        failed = true;
        await context.close();
        continue;
      }
      await tab.evaluate(() => document.fonts.ready);
      // Two frames a beat apart the same: what is drawn has settled.
      let shot = await tab.screenshot({ fullPage: false });
      for (let attempt = 0; attempt < 30; attempt += 1) {
        await tab.waitForTimeout(250);
        const again = await tab.screenshot({ fullPage: false });
        if (again.equals(shot)) break;
        shot = again;
        if (attempt === 29) console.error(`${name}: still changing after 7.5 s; shot as it was`);
      }
      const layout = await tab.evaluate(() => {
        const state = window.__fidelity;
        // Where the browser itself put each cell, to check the gallery's own report against.
        const dom = {};
        for (const id of Object.keys(state.cells)) {
          const node = document.querySelector(`[data-testid="${CSS.escape(id)}"]`);
          if (node) {
            const r = node.getBoundingClientRect();
            dom[id] = [r.x, r.y, r.width, r.height];
          }
        }
        const fonts = [...document.fonts].map((f) => `${f.family} ${f.weight} ${f.status}`);
        return { ...state, dom, fonts };
      });
      layout.errors = errors;
      layout.requested = [...requested].filter((file) => /\.(ttf|woff2?)$/.test(file));
      layout.chromium = browser.version();
      fs.writeFileSync(path.join(spec.out, `${name}.png`), shot);
      fs.writeFileSync(path.join(spec.out, `${name}.json`), JSON.stringify(layout, null, 1));
      console.error(`web ${name}: ${Object.keys(layout.cells).length} cells`);
      await context.close();
    }
  }
} finally {
  await browser.close();
  server.close();
}
process.exit(failed ? 1 : 0);
