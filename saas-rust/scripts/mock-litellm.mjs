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
      const content = `mock:${input}`
      const payload = JSON.stringify({
        id: 'chatcmpl-beta-mock',
        object: 'chat.completion',
        created: 1700000000,
        model: body.model || 'beta-mock',
        choices: [{ index: 0, message: { role: 'assistant', content }, finish_reason: 'stop' }],
        usage: { prompt_tokens: 3, completion_tokens: 4, total_tokens: 7 }
      })
      response.writeHead(200, { 'content-type': 'application/json' }).end(payload)
    } catch {
      response.writeHead(400).end()
    }
  })
})

server.listen(port, '127.0.0.1')
