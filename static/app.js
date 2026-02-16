document.getElementById('healthBtn').addEventListener('click', async () => {
    try {
        const response = await fetch('/api/health');
        const data = await response.json();
        document.getElementById('result').innerHTML = 
            `<strong>健康检查:</strong> ${JSON.stringify(data, null, 2)}`;
    } catch (error) {
        document.getElementById('result').innerHTML = 
            `<strong style="color: red;">错误:</strong> ${error.message}`;
    }
});

document.getElementById('helloBtn').addEventListener('click', async () => {
    try {
        const response = await fetch('/api/hello');
        const data = await response.json();
        document.getElementById('result').innerHTML = 
            `<strong>API 响应:</strong> ${JSON.stringify(data, null, 2)}`;
    } catch (error) {
        document.getElementById('result').innerHTML = 
            `<strong style="color: red;">错误:</strong> ${error.message}`;
    }
});