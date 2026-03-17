const { test, expect } = require('@playwright/test');

test.describe('IT-1 Server Connection', () => {
  test('enter API key -> show connection screen', async ({ page }) => {
    await page.goto('/');
    
    // Fill API key
    await page.fill('input[type="password"]', 'dummy_key');
    await page.click('button:has-text("Save & Continue")');
    
    // Check connection screen
    await expect(page.locator('text=Connect to Hermes Server')).toBeVisible();
    await expect(page.locator('input[type="text"]')).toBeVisible();
  });

  test('paste connection string -> connect and show remote chat', async ({ page }) => {
    await page.goto('/');
    
    // Fill API key
    await page.fill('input[type="password"]', 'dummy_key');
    await page.click('button:has-text("Save & Continue")');
    
    // Fill connection string
    await page.fill('input[type="text"]', 'hermes://mock_pubkey/mock_token?addrs=localhost:8080');
    await page.click('button:has-text("Connect")');
    
    // Check remote chat screen
    await expect(page.locator('text=Remote Server Thread')).toBeVisible();
    await expect(page.locator('button:has-text("Send")')).toBeVisible();
  });
});
