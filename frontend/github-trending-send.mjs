import { chromium } from 'playwright';
import { copyFileSync } from 'fs';
import { join } from 'path';

(async () => {
  console.log('Launching browser...');
  const browser = await chromium.launch({ 
    headless: false,
    slowMo: 100
  });
  console.log('Browser launched');
  
  const page = await browser.newPage();
  console.log('Page created');
  
  await page.goto('https://github.com/trending', { timeout: 60000 });
  console.log('Navigated to GitHub Trending');
  
  // Wait for content to load
  await page.waitForTimeout(5000);
  
  // Take screenshot
  const screenshotPath = 'github_trending.png';
  await page.screenshot({ path: screenshotPath, fullPage: true });
  console.log(`Screenshot saved to: ${screenshotPath}`);
  
  // Copy to workspace
  try {
    const workspacePath = 'C:\\Users\\PANDA-JSR\\.openclaw\\workspace\\github_trending.png';
    copyFileSync(screenshotPath, workspacePath);
    console.log(`Copied to workspace: ${workspacePath}`);
  } catch (e) {
    console.log('Could not copy to workspace:', e.message);
  }
  
  // Get trending repos
  const repos = await page.evaluate(() => {
    const items = document.querySelectorAll('article.Box-row');
    return Array.from(items).slice(0, 10).map(item => {
      const link = item.querySelector('h2 a');
      const desc = item.querySelector('p');
      const stars = item.querySelector('[href$="stargazers"]');
      return {
        name: link ? link.textContent.trim() : 'N/A',
        description: desc ? desc.textContent.trim() : '',
        stars: stars ? stars.textContent.trim() : '0'
      };
    });
  });
  
  console.log('\n=== GitHub Trending Repositories ===\n');
  repos.forEach((repo, i) => {
    console.log(`${i + 1}. ${repo.name}`);
    console.log(`   ${repo.description}`);
    console.log(`   ⭐ ${repo.stars}`);
    console.log('');
  });
  
  await browser.close();
  console.log('Done!');
})().catch(err => {
  console.error('Error:', err);
  process.exit(1);
});
