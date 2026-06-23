/**
 * TGBot Telegram SDK Query Node for n8n
 */
const http = require("http");
const https = require("https");

module.exports = {
  displayName: "TGBot Telegram SDK Query",
  name: "TgbotTelegramSdkQuery",
  icon: "file:tgbot.svg",
  group: ["transform"],
  version: 1,
  description: "Query Telegram Bot API metadata, permissions, and message existence evidence",
  defaults: { name: "TGBot Telegram SDK Query", color: "#0088cc" },
  inputs: ["main"],
  outputs: ["main"],
  credentials: [{ name: "tgbotApi", required: true }],
  properties: [
    {
      displayName: "Operation",
      name: "operation",
      type: "options",
      default: "bot_visibility_status",
      options: [
        { name: "Bound Chat Capabilities", value: "bound_chat_capabilities" },
        { name: "Bot Visibility Status", value: "bot_visibility_status" },
        { name: "Chat Capabilities", value: "chat_capabilities" },
        { name: "Message Existence Probe", value: "message_existence_probe" },
      ],
    },
    {
      displayName: "Chat ID",
      name: "chatId",
      type: "string",
      default: "",
      description: "Telegram chat_id/group_id. Leave empty for bound chat operations.",
    },
    {
      displayName: "Telegram Message ID",
      name: "tgMsgId",
      type: "string",
      default: "",
      description: "Required for Message Existence Probe.",
    },
    {
      displayName: "Staging Chat ID",
      name: "stagingChatId",
      type: "string",
      default: "",
      description: "Optional校对群 ID for forwardMessage existence probes.",
    },
    {
      displayName: "Tenant ID",
      name: "tenantId",
      type: "string",
      default: "",
    },
    {
      displayName: "Limit",
      name: "limit",
      type: "number",
      default: 10,
    },
    {
      displayName: "Include Admins",
      name: "includeAdmins",
      type: "boolean",
      default: false,
    },
  ],
  async execute() {
    const items = this.getInputData();
    const creds = await this.getCredentials("tgbotApi");
    const { baseUrl, apiKey } = creds;

    for (let i = 0; i < items.length; i++) {
      const operation = this.getNodeParameter("operation", i);
      const chatId = this.getNodeParameter("chatId", i);
      const tgMsgId = this.getNodeParameter("tgMsgId", i);
      const stagingChatId = this.getNodeParameter("stagingChatId", i);
      const tenantId = this.getNodeParameter("tenantId", i);
      const limit = this.getNodeParameter("limit", i);
      const includeAdmins = this.getNodeParameter("includeAdmins", i);

      const body = {
        operation,
        chat_id: chatId ? Number(chatId) : undefined,
        group_id: chatId ? Number(chatId) : undefined,
        tg_msg_id: tgMsgId ? Number(tgMsgId) : undefined,
        staging_chat_id: stagingChatId ? Number(stagingChatId) : undefined,
        tenant_id: tenantId || undefined,
        limit,
        include_admins: includeAdmins,
      };

      const response = await this.makeRequest({
        url: baseUrl + "/api/agent/tools/telegram-sdk-query",
        method: "POST",
        body,
        headers: { "Authorization": "Bearer " + apiKey, "Content-Type": "application/json" },
      });

      items[i].json = {
        success: response.status === "ok" && response.success !== false,
        data: response.data || {},
        evidence_count: response.evidence_count,
        data_complete: response.data_complete,
        message: response.message || "",
        raw: response,
      };
    }
    return [items];
  },
  makeRequest(options) {
    return new Promise((resolve, reject) => {
      const url = new URL(options.url);
      const client = url.protocol === "http:" ? http : https;
      const req = client.request({
        hostname: url.hostname,
        port: url.port || (url.protocol === "http:" ? 80 : 443),
        path: url.pathname + url.search,
        method: options.method,
        headers: options.headers,
      }, (res) => {
        let data = "";
        res.on("data", c => data += c);
        res.on("end", () => {
          let parsed;
          try { parsed = JSON.parse(data); } catch { parsed = { text: data }; }
          if (res.statusCode < 200 || res.statusCode >= 300) {
            reject(new Error(`TGBot API ${res.statusCode}: ${data}`));
            return;
          }
          resolve(parsed);
        });
      });
      req.on("error", reject);
      if (options.body) req.write(JSON.stringify(options.body));
      req.end();
    });
  },
};
