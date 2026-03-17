const { test, expect } = require('@playwright/test');

test.describe('IT-0 Local Chat', () => {
  test('app loads and shows API key input', async ({ page }) => {
    await page.goto('/');
    await expect(page.locator('text=Welcome to Hermes Alpha-B')).toBeVisible();
    await expect(page.locator('input[type="password"]')).toBeVisible();
  });

  test('enter API key -> chat screen appears', async ({ page }) => {
    await page.goto('/');
    await page.fill('input[type="password"]', 'dummy_key');
    await page.click('button:has-text("Save & Continue")');
    await expect(page.locator('text=General Thread')).toBeVisible();
    await expect(page.locator('input[type="text"]')).toBeVisible();
  });
});
