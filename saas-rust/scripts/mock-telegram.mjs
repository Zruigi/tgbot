import http from 'node:http'

const port = Number(process.env.MOCK_TELEGRAM_PORT || 18092)
const imageBytes = Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=', 'base64')
const calls = []

function multipartField(rawBody, name) {
  const match = rawBody.match(new RegExp(`name="${name}"\\r\\n\\r\\n([^\\r\\n]*)`))
  return match?.[1]
}

function telegramFailure(response) {
  response.writeHead(200, { 'content-type': 'application/json' }).end(JSON.stringify({
    ok: false,
    error_code: 400,
    description: 'Bad Request: beta delivery rejected',
  }))
}

const server = http.createServer((request, response) => {
  if (request.method === 'GET' && request.url === '/health') {
    response.writeHead(200, { 'content-type': 'application/json' }).end('{"status":"ok"}')
    return
  }

  if (request.method === 'GET' && request.url?.startsWith('/v1/accounts/') && request.url.includes('/transactions/trc20')) {
    response.writeHead(200, { 'content-type': 'application/json' }).end('{"data":[]}')
    return
  }

  if (request.method === 'GET' && request.url?.startsWith('/bscscan?')) {
    response.writeHead(200, { 'content-type': 'application/json' }).end('{"status":"1","message":"OK","result":[]}')
    return
  }

  if (request.method === 'GET' && request.url === '/calls') {
    response.writeHead(200, { 'content-type': 'application/json' }).end(JSON.stringify(calls))
    return
  }
  if (request.method === 'DELETE' && request.url === '/calls') {
    calls.length = 0
    response.writeHead(204).end()
    return
  }

  if (request.method === 'GET' && /^\/file\/bot\d+:[^/]+\/photos\/beta\.png$/.test(request.url || '')) {
    response.writeHead(200, {
      'content-type': 'image/png',
      'content-length': imageBytes.length,
    }).end(imageBytes)
    return
  }

  const match = request.url?.match(/^\/bot(\d+):([^/]+)\/(getMe|setWebhook|sendMessage|sendPhoto|editMessageText|getFile|getChat|getChatMember|getChatMemberCount|getChatAdministrators|banChatMember|unbanChatMember|restrictChatMember|promoteChatMember|deleteMessage|answerCallbackQuery)$/)
  if (request.method !== 'POST' || !match) {
    response.writeHead(404).end()
    return
  }
  const [, botId, tokenSecret, method] = match
  if (tokenSecret.includes('rejected')) {
    response.writeHead(401, { 'content-type': 'application/json' })
      .end('{"ok":false,"error_code":401,"description":"Unauthorized"}')
    return
  }

  const chunks = []
  request.on('data', (chunk) => { chunks.push(chunk) })
  request.on('end', () => {
    const rawBody = Buffer.concat(chunks).toString('utf8')
    if (method === 'getMe') {
      response.writeHead(200, { 'content-type': 'application/json' }).end(JSON.stringify({
        ok: true,
        result: {
          id: Number(botId),
          is_bot: true,
          first_name: 'Beta Bot',
          username: `beta_bot_${botId}`,
        },
      }))
      return
    }
    if (method === 'getChat') {
      const body = JSON.parse(rawBody)
      const chatId = Number(body.chat_id)
      calls.push({ bot_id: Number(botId), method, chat_id: chatId })
      response.writeHead(200, { 'content-type': 'application/json' }).end(JSON.stringify({
        ok: true,
        result: {
          id: chatId,
          type: 'supergroup',
          title: `Beta Group ${chatId}`,
          photo: {
            small_file_id: 'beta-group-photo-small',
            small_file_unique_id: 'beta-group-photo-small-unique',
            big_file_id: 'beta-group-photo-big',
            big_file_unique_id: 'beta-group-photo-big-unique',
          },
        },
      }))
      return
    }
    if (method === 'getChatMember') {
      const body = JSON.parse(rawBody)
      const chatId = Number(body.chat_id)
      const userId = Number(body.user_id)
      const status = userId === 30002 ? 'administrator' : userId === 30003 ? 'creator' : userId === 30006 ? 'left' : 'member'
      calls.push({ bot_id: Number(botId), method, chat_id: chatId, user_id: userId })
      response.writeHead(200, { 'content-type': 'application/json' }).end(JSON.stringify({
        ok: true,
        result: {
          status,
          user: { id: userId, is_bot: false, first_name: `Beta User ${userId}` },
        },
      }))
      return
    }
    if (method === 'getChatMemberCount') {
      const body = JSON.parse(rawBody)
      const chatId = Number(body.chat_id)
      calls.push({ bot_id: Number(botId), method, chat_id: chatId })
      response.writeHead(200, { 'content-type': 'application/json' })
        .end(JSON.stringify({ ok: true, result: 42 }))
      return
    }
    if (method === 'getChatAdministrators') {
      const body = JSON.parse(rawBody)
      const chatId = Number(body.chat_id)
      calls.push({ bot_id: Number(botId), method, chat_id: chatId })
      response.writeHead(200, { 'content-type': 'application/json' }).end(JSON.stringify({
        ok: true,
        result: [
          { status: 'creator', user: { id: Number(botId), is_bot: true, first_name: 'Beta Bot' } },
          { status: 'administrator', user: { id: 30002, is_bot: false, first_name: 'Beta Admin' } },
        ],
      }))
      return
    }
    if (['banChatMember', 'unbanChatMember', 'restrictChatMember', 'promoteChatMember'].includes(method)) {
      const body = JSON.parse(rawBody)
      const chatId = Number(body.chat_id)
      const userId = Number(body.user_id)
      calls.push({
        bot_id: Number(botId),
        method,
        chat_id: chatId,
        user_id: userId,
        permissions: body.permissions || null,
        until_date: body.until_date ?? null,
        only_if_banned: body.only_if_banned ?? null,
        can_manage_chat: body.can_manage_chat ?? null,
      })
      if (userId === 30004) {
        telegramFailure(response)
        return
      }
      response.writeHead(200, { 'content-type': 'application/json' })
        .end(JSON.stringify({ ok: true, result: true }))
      return
    }
    if (method === 'deleteMessage') {
      const body = JSON.parse(rawBody)
      const chatId = Number(body.chat_id)
      const messageId = Number(body.message_id)
      calls.push({
        bot_id: Number(botId),
        method,
        chat_id: chatId,
        message_id: messageId,
      })
      response.writeHead(200, { 'content-type': 'application/json' })
        .end(JSON.stringify({ ok: true, result: true }))
      return
    }
    if (method === 'answerCallbackQuery') {
      const body = JSON.parse(rawBody)
      calls.push({
        bot_id: Number(botId),
        method,
        callback_query_id: String(body.callback_query_id || ''),
        text: String(body.text || ''),
        show_alert: Boolean(body.show_alert),
      })
      response.writeHead(200, { 'content-type': 'application/json' })
        .end(JSON.stringify({ ok: true, result: true }))
      return
    }
    if (method === 'editMessageText') {
      let body
      try {
        body = JSON.parse(rawBody)
      } catch {
        response.writeHead(400).end()
        return
      }
      const chatId = Number(body.chat_id)
      const messageId = Number(body.message_id)
      calls.push({
        bot_id: Number(botId),
        method,
        chat_id: chatId,
        message_id: messageId,
        text: String(body.text || ''),
        reply_markup: body.reply_markup || null,
      })
      response.writeHead(200, { 'content-type': 'application/json' }).end(JSON.stringify({
        ok: true,
        result: {
          message_id: messageId,
          from: { id: Number(botId), is_bot: true, first_name: 'Beta Bot', username: 'beta_bot_' + botId },
          chat: { id: chatId, type: chatId < 0 ? 'supergroup' : 'private' },
          date: 1700000000,
          text: String(body.text || ''),
        },
      }))
      return
    }
    if (method === 'sendMessage') {
      let body
      try {
        body = JSON.parse(rawBody)
      } catch {
        response.writeHead(400).end()
        return
      }
      const chatId = Number(body.chat_id)
      calls.push({
        bot_id: Number(botId),
        method,
        chat_id: chatId,
        text: String(body.text || ''),
        reply_markup: body.reply_markup || null,
        reply_to_message_id: body.reply_to_message_id || null,
      })
      if (chatId === -10002 || chatId === 30008 || chatId === 30009) {
        telegramFailure(response)
        return
      }
      response.writeHead(200, { 'content-type': 'application/json' }).end(JSON.stringify({
        ok: true,
        result: {
          message_id: 501,
          from: { id: Number(botId), is_bot: true, first_name: 'Beta Bot', username: `beta_bot_${botId}` },
          chat: { id: chatId, type: chatId < 0 ? 'supergroup' : 'private' },
          date: 1700000000,
          text: String(body.text || ''),
        },
      }))
      return
    }
    if (method === 'sendPhoto') {
      const isJson = String(request.headers['content-type'] || '').startsWith('application/json')
      let chatId
      let caption = ''
      let replyMarkup = null
      let replyToMessageId = null
      if (isJson) {
        try {
          const body = JSON.parse(rawBody)
          chatId = Number(body.chat_id)
          caption = String(body.caption || '')
          replyMarkup = body.reply_markup || null
          replyToMessageId = body.reply_to_message_id || null
        } catch {
          response.writeHead(400).end()
          return
        }
      } else {
        chatId = Number(multipartField(rawBody, 'chat_id'))
        caption = multipartField(rawBody, 'caption') || ''
        const markup = multipartField(rawBody, 'reply_markup')
        replyToMessageId = Number(multipartField(rawBody, 'reply_to_message_id')) || null
        if (markup) {
          try { replyMarkup = JSON.parse(markup) } catch { replyMarkup = markup }
        }
      }
      calls.push({
        bot_id: Number(botId),
        method,
        chat_id: chatId,
        caption,
        reply_markup: replyMarkup,
        reply_to_message_id: replyToMessageId,
        multipart: !isJson,
      })
      if (chatId === -10002) {
        telegramFailure(response)
        return
      }
      response.writeHead(200, { 'content-type': 'application/json' }).end(JSON.stringify({
        ok: true,
        result: {
          message_id: 502,
          from: { id: Number(botId), is_bot: true, first_name: 'Beta Bot', username: `beta_bot_${botId}` },
          chat: { id: chatId, type: chatId < 0 ? 'supergroup' : 'private' },
          date: 1700000001,
          caption,
          photo: [{ file_id: 'beta-photo-file', width: 1, height: 1, file_size: imageBytes.length }],
        },
      }))
      return
    }
    if (method === 'getFile') {
      response.writeHead(200, { 'content-type': 'application/json' }).end(JSON.stringify({
        ok: true,
        result: {
          file_id: 'beta-photo-file',
          file_unique_id: 'beta-photo-unique',
          file_size: imageBytes.length,
          file_path: 'photos/beta.png',
        },
      }))
      return
    }
    try {
      const body = JSON.parse(rawBody)
      const valid = String(body.url || '').endsWith(`/telegram/webhook/${botId}`)
        && String(body.secret_token || '').startsWith('wh_')
      response.writeHead(200, { 'content-type': 'application/json' })
        .end(JSON.stringify({ ok: true, result: valid }))
    } catch {
      response.writeHead(400).end()
    }
  })
})

server.listen(port, '127.0.0.1')
