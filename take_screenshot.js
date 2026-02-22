const { chromium } = require('playwright');

(async () => {
  // Connect to the existing browser
  const browser = await chromium.connectOverCDP('http://localhost:9222');
  const context = browser.contexts()[0];
  const page = context.pages()[0];
  
  // Take screenshot
  await page.screenshot({ path: 'D:/Code/remote-dev/real_screen_test.png', fullPage: true });
  console.log('Screenshot saved to D:/Code/remote-dev/real_screen_test.png');
  
  await browser.close();
})();
