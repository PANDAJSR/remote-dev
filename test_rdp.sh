#!/bin/bash
# RDP连接测试脚本
# 运行10次完整连接测试

BASE_URL="http://127.0.0.1:3002"
TOTAL_TESTS=10
PASSED=0
FAILED=0

echo "========================================"
echo "远程桌面连接测试"
echo "========================================"
echo ""

for i in $(seq 1 $TOTAL_TESTS); do
    echo ""
    echo "--- 测试 #$i ---"
    TEST_FAILED=0
    
    # 1. 获取服务器信息
    echo "Step 1: 获取服务器信息..."
    SERVER_INFO=$(curl -s -w "\n%{http_code}" "${BASE_URL}/api/rdp/info")
    HTTP_CODE=$(echo "$SERVER_INFO" | tail -n1)
    BODY=$(echo "$SERVER_INFO" | head -n-1)
    
    if [ "$HTTP_CODE" != "200" ]; then
        echo "  ✗ 获取服务器信息失败: HTTP $HTTP_CODE"
        TEST_FAILED=1
    else
        echo "  ✓ 服务器信息获取成功"
        echo "  响应: $BODY"
    fi
    
    # 2. 创建会话
    echo "Step 2: 创建会话..."
    SESSION_RESPONSE=$(curl -s -w "\n%{http_code}" -X POST "${BASE_URL}/api/rdp/session" \
        -H "Content-Type: application/json" \
        -d '{"resolution": [1920, 1080], "fps": 30}')
    HTTP_CODE=$(echo "$SESSION_RESPONSE" | tail -n1)
    BODY=$(echo "$SESSION_RESPONSE" | head -n-1)
    
    if [ "$HTTP_CODE" != "200" ]; then
        echo "  ✗ 创建会话失败: HTTP $HTTP_CODE"
        TEST_FAILED=1
    else
        echo "  ✓ 会话创建成功"
        echo "  响应: $BODY"
    fi
    
    # 3. 检查服务器健康状态
    echo "Step 3: 检查健康状态..."
    HEALTH=$(curl -s -w "\n%{http_code}" "${BASE_URL}/api/health")
    HTTP_CODE=$(echo "$HEALTH" | tail -n1)
    
    if [ "$HTTP_CODE" != "200" ]; then
        echo "  ✗ 健康检查失败: HTTP $HTTP_CODE"
        TEST_FAILED=1
    else
        echo "  ✓ 服务器健康"
    fi
    
    if [ $TEST_FAILED -eq 0 ]; then
        echo "测试 #$i: ✓ 通过"
        PASSED=$((PASSED + 1))
    else
        echo "测试 #$i: ✗ 失败"
        FAILED=$((FAILED + 1))
    fi
    
    # 短暂延迟，避免过于频繁的请求
    sleep 1
done

echo ""
echo "========================================"
echo "测试结果汇总"
echo "========================================"
echo "总测试次数: $TOTAL_TESTS"
echo "通过: $PASSED"
echo "失败: $FAILED"
echo "成功率: $(($PASSED * 100 / $TOTAL_TESTS))%"
echo ""

if [ $FAILED -eq 0 ]; then
    echo "✓ 所有测试通过！成功率 100%"
    exit 0
else
    echo "✗ 存在失败的测试，需要进一步修复"
    exit 1
fi
