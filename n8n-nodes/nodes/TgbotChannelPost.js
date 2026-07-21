/**
 * TGBot Channel Post Node
 */
const http = require("http");
const https = require("https");

module.exports = {
  displayName: "TGBot Channel Post",
  name: "TgbotChannelPost",
  group: ["communication"],
  version: 1,
  description: "Post message to Telegram channel",
  defaults: { name: "TGBot Channel Post", color: "#ff3300" },
  inputs: ["main"],
  outputs: ["main"],
  credentials: [{ name: "tgbotApi", required: true }],
  properties: [
    {
      displayName: "Channel ID",
      name: "channelId",
      type: "string",
      default: "",
      required: true,
      placeholder: "-1001234567890",
    },
    {
      displayName: "Message",
      name: "message",
      type: "string",
      typeOptions: { rows: 4 },
      default: "",
      required: true,
    },
    {
      displayName: "Parse Mode",
      name: "parseMode",
      type: "options",
      options: [
        { name: "None", value: "none" },
        { name: "Markdown", value: "Markdown" },
        { name: "HTML", value: "HTML" },
      ],
      default: "HTML",
    },
  ],
  async execute() {
    const items = this.getInputData();
    const creds = await this.getCredentials("tgbotApi");
    const { baseUrl, apiKey } = creds;
    
    for (let i = 0; i < items.length; i++) {
      const channelId = this.getNodeParameter("channelId", i);
      const message = this.getNodeParameter("message", i);
      const parseMode = this.getNodeParameter("parseMode", i);
      
      const response = await this.makeRequest({
        url: baseUrl + "/api/broadcast/unified",
        method: "POST",
        body: { chat_ids: [Number(channelId)], content: message, parse_mode: parseMode, retry_count: 1 },
        headers: { "Authorization": "Bearer " + apiKey, "Content-Type": "application/json" },
      });
      
      const summary = response.summary || {};
      items[i].json = {
        success: response.status === "ok" && (summary.failed || 0) === 0,
        chat_id: Number(channelId),
        summary,
        failed: response.failed_items || summary.failed_items || [],
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
          try { parsed = JSON.parse(data); } catch { parsed = {}; }
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
