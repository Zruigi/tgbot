/**
 * TGBot Broadcast Node
 */
const http = require("http");
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
  async execute() {
    const items = this.getInputData();
    const creds = await this.getCredentials("tgbotApi");
    const { baseUrl, apiKey } = creds;
    
    for (let i = 0; i < items.length; i++) {
      const targetType = this.getNodeParameter("targetType", i);
      const targetId = this.getNodeParameter("targetId", i);
      const message = this.getNodeParameter("message", i);
      
      const targets = await this.resolveTargets(baseUrl, apiKey, targetType, targetId);
      const body = {
        content: message,
        user_ids: targets.user_ids,
        chat_ids: targets.chat_ids,
        retry_count: 1,
      };
      
      const response = await this.makeRequest({
        url: baseUrl + "/api/broadcast/unified", method: "POST", body: body,
        headers: { "Authorization": "Bearer " + apiKey, "Content-Type": "application/json" },
      });
      
      const summary = response.summary || {};
      items[i].json = {
        success: response.status === "ok",
        sent_count: summary.success || 0,
        failed: response.failed_items || summary.failed_items || [],
        summary,
      };
    }
    return [items];
  },
  async resolveTargets(baseUrl, apiKey, targetType, targetId) {
    if (targetType === "user") {
      if (!targetId) throw new Error("Target ID is required for Specific User");
      return { user_ids: [Number(targetId)], chat_ids: [] };
    }
    if (targetType === "group") {
      if (!targetId) throw new Error("Target ID is required for Specific Group");
      return { user_ids: [], chat_ids: [Number(targetId)] };
    }
    if (targetType === "users") {
      const response = await this.makeRequest({
        url: baseUrl + "/api/users",
        method: "GET",
        headers: { "Authorization": "Bearer " + apiKey, "Content-Type": "application/json" },
      });
      const userIds = (response.data || [])
        .map(u => Number(u.user_id || u.userid || u.id))
        .filter(Boolean);
      return { user_ids: [...new Set(userIds)], chat_ids: [] };
    }

    const response = await this.makeRequest({
      url: baseUrl + "/api/broadcast/targets?chat_types=group,supergroup,channel",
      method: "GET",
      headers: { "Authorization": "Bearer " + apiKey, "Content-Type": "application/json" },
    });
    const chatIds = (response.data || [])
      .map(t => Number(t.chat_id || t.id))
      .filter(Boolean);
    return { user_ids: [], chat_ids: [...new Set(chatIds)] };
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
