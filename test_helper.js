// Test script for 10 iterations
const tests = [];

async function runTest(page, testNum) {
  await page.goto('http://localhost:5137');
  await page.waitForTimeout(2000);
  
  const rdpTab = await page.locator('div').filter({ hasText: /^远程桌面$/ }).nth(4);
  await rdpTab.click();
  
  await page.waitForTimeout(3000);
  
  const connected = await page.locator('text=已连接').isVisible().catch(() => false);
  await page.screenshot({ path: `test_${testNum}.png`, fullPage: true });
  
  return { test: testNum, connected };
}

module.exports = { runTest };