import asyncio
import websockets
import json
import time
import sys

async def test_terminal_ws_connection(uri, connection_id):
    """测试单个 WebSocket 终端连接"""
    try:
        print(f"[{connection_id}] 正在连接...")
        async with websockets.connect(uri) as websocket:
            print(f"[{connection_id}] 已连接")
            
            # 发送一些输入命令
            for i in range(3):
                msg = {
                    "type": "input",
                    "data": f"echo 'Test {connection_id}-{i}'\r\n"
                }
                await websocket.send(json.dumps(msg))
                await asyncio.sleep(0.5)
            
            # 接收一些输出
            try:
                for _ in range(5):
                    response = await asyncio.wait_for(websocket.recv(), timeout=2.0)
                    data = json.loads(response)
                    if data.get("type") == "output":
                        pass  # 忽略输出内容
            except asyncio.TimeoutError:
                pass
            
            print(f"[{connection_id}] 准备关闭连接")
        
        print(f"[{connection_id}] 连接已关闭")
        return True
        
    except Exception as e:
        print(f"[{connection_id}] 错误: {e}")
        return False

async def run_stress_test(uri, num_connections=10):
    """运行压力测试 - 连续打开和关闭多个连接"""
    print(f"开始压力测试: {num_connections} 次连接")
    print(f"WebSocket URI: {uri}")
    print("=" * 60)
    
    success_count = 0
    fail_count = 0
    
    for i in range(1, num_connections + 1):
        print(f"\n--- 第 {i}/{num_connections} 次连接 ---")
        
        if await test_terminal_ws_connection(uri, i):
            success_count += 1
        else:
            fail_count += 1
        
        # 短暂间隔，模拟用户行为
        await asyncio.sleep(1)
    
    print("\n" + "=" * 60)
    print(f"测试完成! 成功: {success_count}, 失败: {fail_count}")
    print("\n请在 Windows 任务管理器中检查 conhost.exe 进程数量")
    print("如果修复有效，conhost.exe 数量应该保持稳定（不会持续增长）")

if __name__ == "__main__":
    # 默认连接到本地后端
    uri = "ws://localhost:3000/ws"
    
    if len(sys.argv) > 1:
        uri = sys.argv[1]
    
    num_connections = 10
    if len(sys.argv) > 2:
        num_connections = int(sys.argv[2])
    
    print("WebSocket 终端压力测试工具")
    print("=" * 60)
    print(f"目标: {uri}")
    print(f"连接次数: {num_connections}")
    print()
    
    try:
        asyncio.run(run_stress_test(uri, num_connections))
    except KeyboardInterrupt:
        print("\n测试被用户中断")
    except Exception as e:
        print(f"\n测试出错: {e}")
