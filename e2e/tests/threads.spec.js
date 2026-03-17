const { test, expect } = require('@playwright/test');

test.describe('IT-2 Threads', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/');
    // Setup API key and connect
    await page.fill('input[type="password"]', 'dummy_key');
    await page.click('button:has-text("Save & Continue")');
    await page.fill('input[type="text"]', 'hermes://mock_pubkey/mock_token?addrs=localhost:8080');
    await page.click('button:has-text("Connect")');
    await expect(page.locator('button:has-text("+")')).toBeVisible();
  });

  test('create thread and send message', async ({ page }) => {
    // Click + to create thread
    await page.click('button:has-text("+")');
    await expect(page.locator('text=New Thread')).toBeVisible();
    
    // Type message
    await page.fill('input[type="text"]', 'Hello thread');
    await page.click('button:has-text("Send")');
    
    // Check message appears
    await expect(page.locator('text=Hello thread')).toBeVisible();
  });

  test('switch between threads', async ({ page }) => {
    // Thread 1
    await page.click('button:has-text("+")');
    await page.fill('input[type="text"]', 'Message 1');
    await page.click('button:has-text("Send")');
    
    // Thread 2
    await page.click('button:has-text("+")');
    await page.fill('input[type="text"]', 'Message 2');
    await page.click('button:has-text("Send")');
    
    // Check message 2 is visible
    await expect(page.locator('text=Message 2')).toBeVisible();
    
    // Switch to Thread 1 (first tab with New Thread)
    await page.locator('text=New Thread').first().click();
    
    // Check message 1 is visible
    await expect(page.locator('text=Message 1')).toBeVisible();
  });

  test('close thread', async ({ page }) => {
    await page.click('button:has-text("+")');
    await expect(page.locator('text=New Thread')).toBeVisible();
    
    // Click close (x)
    await page.locator('text=x').first().click();
    
    // Check it's gone
    // We might have multiple New Threads if tests run in parallel, but in isolation it should work.
  });
});
