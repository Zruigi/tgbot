import http from 'node:http'

const port = Number(process.env.MOCK_LITELLM_PORT || 18091)

const server = http.createServer((request, response) => {
  if (request.method === 'GET' && request.url === '/health') {
    response.writeHead(200, { 'content-type': 'application/json' }).end('{"status":"ok"}')
    return
  }
  if (request.method !== 'POST' || request.url !== '/v1/chat/completions') {
    response.writeHead(404).end()
    return
  }

  let rawBody = ''
  request.setEncoding('utf8')
  request.on('data', (chunk) => { rawBody += chunk })
  request.on('end', () => {
    try {
      const body = JSON.parse(rawBody)
      const messages = Array.isArray(body.messages) ? body.messages : []
      const input = String(messages.at(-1)?.content || '')
      const systemInput = String(messages.find((message) => message?.role === 'system')?.content || '')
      const intentPrefix = '分析用户意图，返回 JSON 对象：'
      const userMessageMarker = '用户消息: '
      const nativeToolRequest = messages.some((message) =>
        message?.role === 'user' && String(message?.content || '').trim() === 'query clusters beta')
      const hasToolResult = messages.some((message) =>
        String(message?.content || '').includes('[工具执行结果]'))
      let content = `mock:${input}`
      let toolCalls = []
      if (systemInput.includes('You are an ad classifier')) {
        const arrayStart = input.indexOf('[')
        const clusters = arrayStart >= 0 ? JSON.parse(input.slice(arrayStart)) : []
        content = JSON.stringify(clusters.map((cluster) => ({
          family_hash: String(cluster.family_hash || ''),
          is_ad: true,
          confidence: 0.95,
          reason: 'beta runtime deterministic ad classification'
        })))
      } else if (nativeToolRequest && !hasToolResult) {
        content = null
        toolCalls = [{
          id: 'call_beta_query_ad_clusters',
          type: 'function',
          function: {
            name: 'query_ad_clusters',
            arguments: JSON.stringify({ status: 'pending', limit: 20, window_minutes: 60 })
          }
        }]
      } else if (nativeToolRequest && hasToolResult) {
        content = 'mock:query clusters beta'
      } else if (input.startsWith(intentPrefix)) {
        const markerIndex = input.lastIndexOf(userMessageMarker)
        const userMessage = markerIndex >= 0
          ? input.slice(markerIndex + userMessageMarker.length).trim()
          : ''
        content = JSON.stringify({
          need_tool: false,
          tool_name: null,
          group_id: null,
          reply_content: `mock:${userMessage}`
        })
      }
      const payload = JSON.stringify({
        id: 'chatcmpl-beta-mock',
        object: 'chat.completion',
        created: 1700000000,
        model: body.model || 'beta-mock',
        choices: [{
          index: 0,
          message: { role: 'assistant', content, ...(toolCalls.length > 0 ? { tool_calls: toolCalls } : {}) },
          finish_reason: toolCalls.length > 0 ? 'tool_calls' : 'stop'
        }],
        usage: { prompt_tokens: 3, completion_tokens: 4, total_tokens: 7 }
      })
      response.writeHead(200, { 'content-type': 'application/json' }).end(payload)
    } catch {
      response.writeHead(400).end()
    }
  })
})

server.listen(port, '127.0.0.1')
