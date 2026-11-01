import { expect, test } from '@playwright/test';
import { mkdirSync } from 'node:fs';
import { resolve } from 'node:path';

const artifacts = resolve(process.cwd(), '..', 'artifacts');
mkdirSync(artifacts, { recursive: true });

function screenshotPath(projectName: string, suffix = ''): string {
  const base = projectName === 'mobile-chromium' ? 'webui-mobile' : 'webui';
  return resolve(artifacts, `${base}${suffix}.png`);
}

test('renders the topology, inspects a node, and opens the mutation diff', async ({
  page,
}, testInfo) => {
  await page.goto('/');

  await expect(page.getByTestId('status-bar')).toBeVisible();
  await expect(page.getByTestId('topbar-connection')).toContainText('Live');
  await expect(page.locator('[data-node-name="hello"]')).toBeVisible({ timeout: 10_000 });

  const shells = page.locator('[data-testid="node-shell"]');
  await expect(shells.first()).toBeVisible();
  expect(await shells.count()).toBeGreaterThanOrEqual(7);

  const overflow = await page.evaluate(
    () => document.documentElement.scrollWidth - window.innerWidth,
  );
  expect(overflow).toBeLessThanOrEqual(1);

  await page.locator('[data-node-name="hello"]').click();
  const inspector = page.getByTestId('inspector-drawer');
  await expect(inspector).toBeVisible();
  await expect(inspector.getByRole('heading', { name: 'hello', level: 2 })).toBeVisible();

  await inspector.getByRole('tab', { name: 'Mutations' }).click();
  await expect(inspector.getByText('Three-way mutation diff')).toBeVisible();
  await expect(inspector.getByText('User intent')).toBeVisible();
  await expect(inspector.getByText('Knative defaults')).toBeVisible();
  await expect(inspector.getByText('Policy mutators')).toBeVisible();
  await expect(inspector.getByText('knative-serving-controller').first()).toBeVisible();

  await inspector.getByRole('tab', { name: 'Policies' }).click();
  await expect(inspector.getByText('missing the owner label').first()).toBeVisible();

  await inspector.getByRole('tab', { name: 'YAML' }).click();
  await expect(inspector.getByText('redacted')).toBeVisible();
  await expect(inspector.locator('pre.yaml')).toContainText('apiVersion: serving.knative.dev/v1');

  if (testInfo.project.name === 'mobile-chromium') {
    await page.keyboard.press('Escape');
    await expect(inspector).toBeHidden();
    await page.screenshot({ path: screenshotPath(testInfo.project.name) });
  } else {
    await page.screenshot({ path: screenshotPath(testInfo.project.name) });
    await page.keyboard.press('Escape');
    await expect(inspector).toBeHidden();
  }

  await page.locator('[data-node-name="orders"]').click();
  await expect(inspector.getByRole('heading', { name: 'orders', level: 2 })).toBeVisible();
  await inspector.getByRole('button', { name: 'Open tap' }).click();

  const tapPanel = page.getByTestId('tap-drawer');
  await expect(tapPanel).toBeVisible();
  await expect(tapPanel.getByText('Live tap')).toBeVisible();
  await tapPanel.getByRole('button', { name: 'Start tap' }).click();
  await expect(tapPanel.getByText('order.created').first()).toBeVisible({ timeout: 10_000 });
  await expect(tapPanel.locator('.event-list li')).toHaveCount(1);

  await page.screenshot({ path: screenshotPath(testInfo.project.name, '-tap') });
});

test('shows a clear failure state when the RPC endpoint fails', async ({ page }) => {
  await page.route('**/rpc', (route) =>
    route.fulfill({ status: 500, contentType: 'application/json', body: '{}' }),
  );
  await page.goto('/');

  await expect(page.getByRole('heading', { name: 'Cannot reach the daemon' })).toBeVisible();
  await expect(page.getByText(/rpc daemon\.status failed with HTTP 500/)).toBeVisible();
  await expect(page.getByRole('button', { name: 'Retry connection' })).toBeVisible();

  await page.unroute('**/rpc');
  await page.getByRole('button', { name: 'Retry connection' }).click();
  await expect(page.locator('[data-node-name="hello"]')).toBeVisible({ timeout: 10_000 });
});
