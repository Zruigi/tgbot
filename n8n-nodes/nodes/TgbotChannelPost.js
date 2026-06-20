/**
 * TGBot Channel Post Node
 */
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
  async execute(this) {
    const items = this.getInputData();
    const creds = await this.getCredentials("tgbotApi");
    const { baseUrl, apiKey } = creds;
    
    for (let i = 0; i < items.length; i++) {
      const channelId = this.getNodeParameter("channelId", i);
      const message = this.getNodeParameter("message", i);
      const parseMode = this.getNodeParameter("parseMode", i);
      
      const response = await this.makeRequest({
        url: baseUrl + "/api/channel/post",
        method: "POST",
        body: { channel_id: channelId, message: message, parse_mode: parseMode },
        headers: { "Authorization": "Bearer " + apiKey, "Content-Type": "application/json" },
      });
      
      items[i].json = { success: true, message_id: response.message_id, chat_id: response.chat_id };
    }
    return [items];
  },
  makeRequest(options) {
    return new Promise((resolve, reject) => {
      const url = new URL(options.url);
      const req = https.request({
        hostname: url.hostname, port: 443, path: url.pathname, method: options.method, headers: options.headers,
      }, (res) => {
        let data = "";
        res.on("data", c => data += c);
        res.on("end", () => { try { resolve(JSON.parse(data)); } catch { resolve({}); } });
      });
      req.on("error", reject);
      if (options.body) req.write(JSON.stringify(options.body));
      req.end();
    });
  },
};
