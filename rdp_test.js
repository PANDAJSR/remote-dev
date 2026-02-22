const { chromium } = require('playwright');
const fs = require('fs');
const path = require('path');

const TEST_COUNT = 10;
const WAIT_TIME = 8000; // 8 seconds for WebRTC connection
const RESULTS_FILE = 'D:\\Code\\remote-dev\\test_results.json';
const SCREENSHOT_DIR = 'D:\\Code\\remote-dev';

async function runTest(testNumber) {
    console.log(`Running test ${testNumber}/${TEST_COUNT}...`);
    
    const browser = await chromium.launch({ headless: false });
    const context = await browser.newContext({
        viewport: { width: 1280, height: 720 }
    });
    const page = await context.newPage();
    
    const result = {
        testNumber,
        timestamp: new Date().toISOString(),
        success: false,
        screenshotPath: null,
        error: null,
        videoDetected: false
    };
    
    try {
        // Navigate to the frontend
        await page.goto('http://localhost:5137', { timeout: 30000 });
        
        // Wait for the page to load
        await page.waitForLoadState('networkidle');
        
        // Wait 8 seconds for WebRTC connection
        console.log(`  Waiting ${WAIT_TIME}ms for WebRTC connection...`);
        await page.waitForTimeout(WAIT_TIME);
        
        // Take screenshot
        const screenshotPath = path.join(SCREENSHOT_DIR, `rdp_test_${String(testNumber).padStart(2, '0')}.png`);
        await page.screenshot({ path: screenshotPath, fullPage: false });
        result.screenshotPath = screenshotPath;
        console.log(`  Screenshot saved: ${screenshotPath}`);
        
        // Check for video element and if it's displaying content (not black)
        const videoCheck = await page.evaluate(() => {
            const video = document.querySelector('video');
            if (!video) return { hasVideo: false, reason: 'No video element found' };
            
            // Check if video is playing and has dimensions
            const hasDimensions = video.videoWidth > 0 && video.videoHeight > 0;
            const isPlaying = !video.paused && video.readyState >= 2;
            
            return {
                hasVideo: true,
                videoWidth: video.videoWidth,
                videoHeight: video.videoHeight,
                isPlaying,
                readyState: video.readyState,
                currentTime: video.currentTime
            };
        });
        
        result.videoInfo = videoCheck;
        
        // Determine success: video element exists and has dimensions
        if (videoCheck.hasVideo && videoCheck.videoWidth > 0 && videoCheck.videoHeight > 0) {
            result.success = true;
            result.videoDetected = true;
            console.log(`  ✓ Test ${testNumber} PASSED - Video detected (${videoCheck.videoWidth}x${videoCheck.videoHeight})`);
        } else {
            result.success = false;
            result.error = videoCheck.reason || 'Video not properly loaded';
            console.log(`  ✗ Test ${testNumber} FAILED - ${result.error}`);
        }
        
    } catch (error) {
        result.success = false;
        result.error = error.message;
        console.log(`  ✗ Test ${testNumber} FAILED - Error: ${error.message}`);
        
        // Try to take screenshot even on error
        try {
            const screenshotPath = path.join(SCREENSHOT_DIR, `rdp_test_${String(testNumber).padStart(2, '0')}_error.png`);
            await page.screenshot({ path: screenshotPath, fullPage: false });
            result.screenshotPath = screenshotPath;
        } catch (screenshotError) {
            console.log(`  Could not take error screenshot: ${screenshotError.message}`);
        }
    }
    
    await browser.close();
    return result;
}

async function main() {
    console.log('========================================');
    console.log('Remote Desktop Connection Test');
    console.log('========================================\n');
    
    const results = [];
    let passedCount = 0;
    let failedCount = 0;
    
    for (let i = 1; i <= TEST_COUNT; i++) {
        const result = await runTest(i);
        results.push(result);
        
        if (result.success) {
            passedCount++;
        } else {
            failedCount++;
        }
        
        // Small delay between tests
        if (i < TEST_COUNT) {
            console.log('  Waiting 2 seconds before next test...\n');
            await new Promise(resolve => setTimeout(resolve, 2000));
        }
    }
    
    // Generate report
    console.log('\n========================================');
    console.log('Test Results Summary');
    console.log('========================================');
    console.log(`Total Tests: ${TEST_COUNT}`);
    console.log(`Passed: ${passedCount}`);
    console.log(`Failed: ${failedCount}`);
    console.log(`Success Rate: ${(passedCount / TEST_COUNT * 100).toFixed(1)}%`);
    console.log('========================================\n');
    
    // List failed tests
    const failedTests = results.filter(r => !r.success);
    if (failedTests.length > 0) {
        console.log('Failed Tests:');
        failedTests.forEach(test => {
            console.log(`  Test ${test.testNumber}: ${test.error}`);
        });
        console.log('');
    }
    
    // Save results to JSON
    const report = {
        timestamp: new Date().toISOString(),
        summary: {
            totalTests: TEST_COUNT,
            passed: passedCount,
            failed: failedCount,
            successRate: `${(passedCount / TEST_COUNT * 100).toFixed(1)}%`
        },
        results: results
    };
    
    fs.writeFileSync(RESULTS_FILE, JSON.stringify(report, null, 2));
    console.log(`Results saved to: ${RESULTS_FILE}`);
    
    process.exit(failedCount > 0 ? 1 : 0);
}

main().catch(error => {
    console.error('Fatal error:', error);
    process.exit(1);
});
