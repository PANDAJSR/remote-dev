const { chromium } = require('playwright');

async function runBrowserTest() {
    const url = process.argv[2] || 'http://localhost:3000';
    const iterations = parseInt(process.argv[3]) || 10;
    
    console.log('Playwright 浏览器测试工具');
    console.log('========================');
    console.log(`目标URL: ${url}`);
    console.log(`迭代次数: ${iterations}`);
    console.log();
    
    console.log('请在测试前记录当前 conhost.exe 进程数量');
    console.log('测试完成后再次检查，数量应该保持稳定');
    console.log();
    
    const browser = await chromium.launch({ headless: false });
    
    try {
        for (let i = 1; i <= iterations; i++) {
            console.log(`\n--- 第 ${i}/${iterations} 次测试 ---`);
            
            const context = await browser.newContext();
            const page = await context.newPage();
            
            try {
                // 打开页面
                console.log('正在打开页面...');
                await page.goto(url, { waitUntil: 'networkidle', timeout: 30000 });
                
                // 等待页面加载和 WebSocket 连接建立
                await page.waitForTimeout(3000);
                
                // 在终端中输入一些命令
                console.log('发送测试命令到终端...');
                await page.keyboard.type('echo "Hello from test"');
                await page.keyboard.press('Enter');
                await page.waitForTimeout(1000);
                
                await page.keyboard.type('pwd');
                await page.keyboard.press('Enter');
                await page.waitForTimeout(1000);
                
                console.log('准备关闭页面...');
                
            } catch (e) {
                console.error(`第 ${i} 次测试出错:`, e.message);
            } finally {
                // 关闭页面和上下文
                await page.close();
                await context.close();
                console.log('页面已关闭');
            }
            
            // 等待一下再进行下一次
            await new Promise(resolve => setTimeout(resolve, 2000));
        }
        
        console.log('\n========================');
        console.log('所有测试完成!');
        console.log('\n请检查 conhost.exe 进程数量：');
        console.log('1. 打开任务管理器');
        console.log('2. 查看详细信息选项卡');
        console.log('3. 查找 conhost.exe 进程');
        console.log('4. 如果修复有效，数量应该保持稳定');
        
    } finally {
        await browser.close();
        console.log('\n浏览器已关闭');
    }
}

runBrowserTest().catch(console.error);
