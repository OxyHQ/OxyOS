import { afterAll, beforeAll, describe, expect, test } from 'bun:test';
import { chromium, type Browser } from 'playwright';
import { preview, type PreviewServer } from 'vite';
import type { AddressInfo } from 'node:net';

let server: PreviewServer;
let browser: Browser;
let origin: string;

beforeAll(async () => {
  const build = Bun.spawn(
    [
      'bunx',
      'vite',
      'build',
      '--config',
      'vite.web.config.ts',
      '--outDir',
      'node_modules/.cache/platform-browser',
    ],
    {
      cwd: new URL('..', import.meta.url).pathname,
      env: {
        ...process.env,
        NODE_ENV: 'production',
      },
      stdout: 'pipe',
      stderr: 'pipe',
    },
  );
  const [stdout, stderr, code] = await Promise.all([
    new Response(build.stdout).text(),
    new Response(build.stderr).text(),
    build.exited,
  ]);
  if (code !== 0) throw new Error(`Browser fixture build failed: ${stdout}\n${stderr}`);
  server = await preview({
    configFile: false,
    root: new URL('..', import.meta.url).pathname,
    build: { outDir: 'node_modules/.cache/platform-browser' },
    preview: { host: '127.0.0.1', port: 0 },
  });
  origin = `http://127.0.0.1:${(server.httpServer.address() as AddressInfo).port}`;
  browser = await chromium.launch({ headless: true });
}, 60_000);

afterAll(async () => {
  await browser?.close();
  if (server) await new Promise<void>((resolve) => server.httpServer.close(() => resolve()));
});

describe('the desktop shared account and Alia integration', () => {
  test('opens the shared sign-in dialog without fabricating a bearer or requesting app permissions', async () => {
    const page = await browser.newPage();
    const errors: string[] = [];
    const bearerRequests: string[] = [];
    page.on('pageerror', (error) => errors.push(error.message));
    // A public registry fixture only. No signed-in account or token is invented.
    await page.route('https://api.oxy.so/**', (route) => {
      const request = route.request();
      if (request.headers().authorization) bearerRequests.push(request.url());
      const publicClient = request
        .url()
        .endsWith('/auth/oauth/client/oxy_dk_ee7af26b21c2ff9e70f59e43dc2359106280665d779b74a8');
      return route.fulfill({
        status: publicClient ? 200 : 401,
        contentType: 'application/json',
        body: JSON.stringify(
          publicClient
            ? {
                application: {
                  id: 'test-oxyos',
                  name: 'OxyOS browser fixture',
                  type: 'first_party',
                  isOfficial: true,
                  isInternal: false,
                  scopes: [],
                },
              }
            : { error: 'unauthenticated' },
        ),
      });
    });
    await page.route('https://api.alia.onl/**', (route) => {
      if (route.request().headers().authorization) bearerRequests.push(route.request().url());
      return route.fulfill({ status: 401, contentType: 'application/json', body: '{}' });
    });
    try {
      await page.goto(`${origin}/web.html`);
      // The Linux lock screen remains separate from the Oxy account dialog.
      await page
        .locator('input[type=password]')
        .waitFor({ state: 'visible', timeout: 5000 })
        .catch(async (error) => {
          throw new Error(
            `${String(error)}\nPage errors: ${JSON.stringify(errors)}\nBody: ${await page.locator('body').innerText()}`,
          );
        });
      await page.locator('input[type=password]').fill('1234');
      await page.locator('input[type=password]').press('Enter');
      await page.getByRole('button', { name: 'Open Alia' }).click();
      await page.getByRole('button', { name: 'Account', exact: true }).click();
      await page.getByRole('dialog').waitFor({ state: 'visible' });
      expect(await page.getByRole('dialog').innerText()).toContain('Email or username');
      expect(await page.getByRole('dialog').innerText()).not.toMatch(
        /allow .*access|scopes|permissions/i,
      );
      expect(bearerRequests).toEqual([]);
      expect(errors).toEqual([]);
    } finally {
      await page.close();
    }
  }, 30_000);
});
