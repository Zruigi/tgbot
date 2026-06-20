/**
 * TGBot Broadcast Node
 */
const https = require("https");

module.exports = {
  displayName: "TGBot Broadcast",
  name: "TgbotBroadcast",
  group: ["communication"],
  version: 1,
  description: "Broadcast message to users or groups",
  defaults: { name: "TGBot Broadcast", color: "#9933ff" },
  inputs: ["main"],
  outputs: ["main"],
  credentials: [{ name: "tgbotApi", required: true }],
  properties: [
    {
      displayName: "Target Type",
      name: "targetType",
      type: "options",
      options: [
        { name: "All Users", value: "users" },
        { name: "All Groups", value: "groups" },
        { name: "Specific User", value: "user" },
        { name: "Specific Group", value: "group" },
      ],
      default: "users",
    },
    {
      displayName: "Target ID",
      name: "targetId",
      type: "string",
      default: "",
      placeholder: "User or Group ID (for specific target)",
    },
    {
      displayName: "Message",
      name: "message",
      type: "string",
      typeOptions: { rows: 4 },
      default: "",
      required: true,
    },
  ],
  async execute(this) {
    const items = this.getInputData();
    const creds = await this.getCredentials("tgbotApi");
    const { baseUrl, apiKey } = creds;
    
    for (let i = 0; i < items.length; i++) {
      const targetType = this.getNodeParameter("targetType", i);
      const targetId = this.getNodeParameter("targetId", i);
      const message = this.getNodeParameter("message", i);
      
      let url = baseUrl + "/api/broadcast/unified";
      const body = {
        target_type: targetType,
        target_id: targetId || undefined,
        content: message,
      };
      
      const response = await this.makeRequest({
        url: url, method: "POST", body: body,
        headers: { "Authorization": "Bearer " + apiKey, "Content-Type": "application/json" },
      });
      
      items[i].json = { success: true, sent_count: response.sent_count || 0, failed: response.failed || [] };
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
