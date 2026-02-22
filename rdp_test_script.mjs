import { chromium } from 'playwright';
import fs from 'fs';
import path from 'path';

const TEST_URL = 'http://localhost:5137';
const TEST_COUNT = 10;
const SCREENSHOT_DIR = 'rdp_test_results';

// Create directory for screenshots
if (!fs.existsSync(SCREENSHOT_DIR)) {
  fs.mkdirSync(SCREENSHOT_DIR);
}

const results = [];

async function runTest(iteration) {
  console.log(`\n========== Test ${iteration}/${TEST_COUNT} ==========`);
  
  const browser = await chromium.launch({ headless: false });
  const context = await browser.newContext({
    viewport: { width: 1920, height: 1080 }
  });
  const page = await context.newPage();
  
  try {
    // Navigate to the page
    console.log(`[${iteration}] Navigating to ${TEST_URL}...`);
    await page.goto(TEST_URL, { waitUntil: 'networkidle', timeout: 30000 });
    
    // Wait for page to load
    await page.waitForTimeout(3000);
    
    // Click on "远程桌面" tab
    console.log(`[${iteration}] Clicking on 远程桌面 tab...`);
    const rdpTab = await page.locator('div').filter({ hasText: /^远程桌面$/ }).nth(4);
    await rdpTab.click();
    
    // Wait for connection
    console.log(`[${iteration}] Waiting for connection...`);
    await page.waitForTimeout(5000);
    
    // Get connection status
    const status = await page.locator('text=已连接').isVisible().catch(() => false);
    console.log(`[${iteration}] Connection status: ${status ? 'Connected' : 'Not connected'}`);
    
    // Take screenshot
    const screenshotPath = path.join(SCREENSHOT_DIR, `test_${iteration}.png`);
    console.log(`[${iteration}] Taking screenshot: ${screenshotPath}`);
    await page.screenshot({ path: screenshotPath, fullPage: true });
    
    // Check for video element
    const videoElement = await page.locator('video').first();
    const videoVisible = await videoElement.isVisible().catch(() => false);
    const videoSrc = await videoElement.getAttribute('src').catch(() => null);
    
    console.log(`[${iteration}] Video visible: ${videoVisible}`);
    console.log(`[${iteration}] Video srcObject set: ${videoSrc ? 'Yes' : 'No'}`);
    
    // Get console errors
    const errors = [];
    page.on('console', msg => {
      if (msg.type() === 'error') {
        errors.push(msg.text());
      }
    });
    
    await page.waitForTimeout(1000);
    
    results.push({
      iteration,
      timestamp: new Date().toISOString(),
      connected: status,
      videoVisible,
      screenshotPath,
      errors: errors.slice(0, 5) // First 5 errors
    });
    
    console.log(`[${iteration}] Test completed successfully`);
    
  } catch (error) {
    console.error(`[${iteration}] Test failed:`, error.message);
    results.push({
      iteration,
      timestamp: new Date().toISOString(),
      error: error.message,
      screenshotPath: null
    });
  } finally {
    await browser.close();
  }
}

async function main() {
  console.log('Starting Remote Desktop Test Suite');
  console.log(`URL: ${TEST_URL}`);
  console.log(`Test count: ${TEST_COUNT}`);
  console.log('');
  
  for (let i = 1; i <= TEST_COUNT; i++) {
    await runTest(i);
    // Wait between tests
    if (i < TEST_COUNT) {
      console.log(`Waiting 3 seconds before next test...`);
      await new Promise(resolve => setTimeout(resolve, 3000));
    }
  }
  
  // Summary
  console.log('\n========== Test Summary ==========');
  const successful = results.filter(r => !r.error && r.connected).length;
  const failed = results.filter(r => r.error || !r.connected).length;
  
  console.log(`Total tests: ${TEST_COUNT}`);
  console.log(`Successful: ${successful}`);
  console.log(`Failed: ${failed}`);
  console.log('');
  
  // Check for black screen issues
  console.log('Detailed results:');
  results.forEach(r => {
    const status = r.error ? 'ERROR' : (r.connected ? 'PASS' : 'FAIL');
    console.log(`Test ${r.iteration}: ${status} - Connected: ${r.connected}, Video: ${r.videoVisible}`);
    if (r.errors && r.errors.length > 0) {
      console.log(`  Errors: ${r.errors.length}`);
    }
  });
  
  // Save results to file
  fs.writeFileSync(
    path.join(SCREENSHOT_DIR, 'test_results.json'),
    JSON.stringify(results, null, 2)
  );
  
  console.log(`\nResults saved to ${SCREENSHOT_DIR}/test_results.json`);
  
  if (failed > 0) {
    console.log('\n⚠️  Some tests failed. Check screenshots and logs.');
    process.exit(1);
  } else {
    console.log('\n✅ All tests passed!');
    process.exit(0);
  }
}

main().catch(console.error);
