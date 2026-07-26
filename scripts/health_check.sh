#!/bin/bash
# AI 服务健康检查

echo "🏥 健康检查开始..."

# 检查 LiteLLM
if ! sudo docker ps | grep -q litellm; then
    echo "❌ LiteLLM 未运行"
    exit 1
fi

# 检查 API 响应
RESPONSE=$(curl -s -o /dev/null -w "%{http_code}" \
  http://localhost:4000/health)

if [ "$RESPONSE" != "200" ]; then
    echo "❌ LiteLLM 健康检查失败: HTTP $RESPONSE"
    exit 1
fi

# LiteLLM 网关 key：从环境变量读取，不再硬编码
LITELLM_MASTER_KEY="${LITELLM_MASTER_KEY:?请设置 LITELLM_MASTER_KEY 环境变量（LiteLLM 网关 key）}"

# 检查关键模型
MODELS=("gpt-3.5-turbo" "gpt-4" "gemini-3.1-pro-preview")
for model in "${MODELS[@]}"; do
    RESPONSE=$(curl -s -o /dev/null -w "%{http_code}" \
      -H "Authorization: Bearer ${LITELLM_MASTER_KEY}" \
      -H "Content-Type: application/json" \
      -d "{\"model\":\"$model\",\"messages\":[{\"role\":\"user\",\"content\":\"test\"}]}" \
      http://localhost:4000/v1/chat/completions)
    
    if [ "$RESPONSE" != "200" ]; then
        echo "⚠️  模型 $model 不可用: HTTP $RESPONSE"
    else
        echo "✅ 模型 $model 正常"
    fi
done

echo "✅ 健康检查完成"
