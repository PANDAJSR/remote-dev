const { chromium } = require('playwright');
const fs = require('fs');
const path = require('path');

async function runTests() {
  console.log('Starting 10 remote desktop connection tests...');
  
  const results = [];
  const browser = await chromium.launch({ headless: false });
  
  for (let i = 1; i <= 10; i++) {
    console.log(`\n=== Test ${i}/10 ===`);
    
    try {
      const context = await browser.newContext({
        viewport: { width: 1920, height: 1080 }
      });
      const page = await context.newPage();
      
      // Navigate to the frontend
      console.log('Navigating to http://localhost:5137...');
      await page.goto('http://localhost:5137', { timeout: 30000 });
      
      // Wait for connection
      console.log('Waiting for connection...');
      await page.waitForTimeout(8000);
      
      // Take screenshot
      const screenshotPath = `D:\\Code\\remote-dev\\rdp_test_${String(i).padStart(2, '0')}.png`;
      await page.screenshot({ path: screenshotPath, fullPage: false });
      console.log(`Screenshot saved: ${screenshotPath}`);
      
      // Check video status
      const videoInfo = await page.evaluate(() => {
        const video = document.querySelector('.rdp-video');
        const statusEl = document.querySelector('.rdp-status-connected, .rdp-status-error, .rdp-status-connecting');
        
        return {
          hasVideo: !!video,
          videoWidth: video?.videoWidth || 0,
          videoHeight: video?.videoHeight || 0,
          status: statusEl?.textContent?.trim() || 'unknown'
        };
      });
      
      console.log('Video info:', videoInfo);
      
      // Determine if test passed
      const passed = videoInfo.hasVideo && videoInfo.videoWidth > 0 && videoInfo.videoHeight > 0;
      results.push({ test: i, passed, ...videoInfo });
      
      console.log(`Test ${i}: ${passed ? 'PASSED ✓' : 'FAILED ✗'}`);
      
      await context.close();
      
      // Wait between tests
      if (i < 10) {
        console.log('Waiting 3 seconds before next test...');
        await new Promise(r => setTimeout(r, 3000));
      }
      
    } catch (error) {
      console.error(`Test ${i} error:`, error.message);
      results.push({ test: i, passed: false, error: error.message });
    }
  }
  
  await browser.close();
  
  // Print summary
  console.log('\n\n========== TEST SUMMARY ==========');
  const passedCount = results.filter(r => r.passed).length;
  console.log(`Success rate: ${passedCount}/10 (${passedCount * 10}%)`);
  
  results.forEach(r => {
    console.log(`Test ${r.test}: ${r.passed ? '✓ PASS' : '✗ FAIL'} - Status: ${r.status}${r.error ? ' - Error: ' + r.error : ''}`);
  });
  
  // Save results to file
  const resultsPath = 'D:\\Code\\remote-dev\\test_results.json';
  fs.writeFileSync(resultsPath, JSON.stringify(results, null, 2));
  console.log(`\nResults saved to: ${resultsPath}`);
  
  process.exit(passedCount === 10 ? 0 : 1);
}

runTests().catch(err => {
  console.error('Fatal error:', err);
  process.exit(1);
});
