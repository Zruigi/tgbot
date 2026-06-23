/**
 * TGBot Advertisement Cluster Pipeline Node
 */
const http = require("http");
const https = require("https");

module.exports = {
  displayName: "TGBot Ad Cluster",
  name: "TgbotAdCluster",
  group: ["transform"],
  version: 1,
  description: "Query, ingest, backfill, and delete aggregated advertisement clusters",
  defaults: { name: "TGBot Ad Cluster", color: "#e11d48" },
  inputs: ["main"],
  outputs: ["main"],
  credentials: [{ name: "tgbotApi", required: true }],
  properties: [
    {
      displayName: "Operation",
      name: "operation",
      type: "options",
      options: [
        { name: "Query Clusters", value: "query" },
        { name: "Ingest Message Event", value: "ingest" },
        { name: "Backfill From SQLite", value: "backfill" },
        { name: "Delete Cluster", value: "delete" },
      ],
      default: "query",
    },
    {
      displayName: "Tenant ID",
      name: "tenantId",
      type: "string",
      default: "={{ $json.tenant_id || '' }}",
    },
    {
      displayName: "Group / Chat ID",
      name: "chatId",
      type: "string",
      default: "={{ $json.chat_id || $json.group_id || '' }}",
    },
    {
      displayName: "Telegram Message ID",
      name: "tgMsgId",
      type: "string",
      default: "={{ $json.tg_msg_id || '' }}",
    },
    {
      displayName: "Content",
      name: "content",
      type: "string",
      typeOptions: { rows: 3 },
      default: "={{ $json.content || $json.text || '' }}",
    },
    {
      displayName: "Sender ID",
      name: "senderId",
      type: "string",
      default: "={{ $json.sender_user_id || $json.sender_id || '' }}",
    },
    {
      displayName: "Cluster ID",
      name: "clusterId",
      type: "string",
      default: "={{ $json.cluster_id || '' }}",
    },
    {
      displayName: "Status",
      name: "status",
      type: "options",
      options: [
        { name: "Pending", value: "pending" },
        { name: "Ad", value: "ad" },
        { name: "Not Ad", value: "not_ad" },
        { name: "All", value: "all" },
      ],
      default: "pending",
    },
    {
      displayName: "Limit",
      name: "limit",
      type: "number",
      default: 20,
    },
    {
      displayName: "Window Minutes",
      name: "windowMinutes",
      type: "number",
      default: 60,
    },
    {
      displayName: "Dry Run",
      name: "dryRun",
      type: "boolean",
      default: true,
    },
    {
      displayName: "Ban Senders",
      name: "banSenders",
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
      const tenantId = this.getNodeParameter("tenantId", i);
      const chatId = this.getNodeParameter("chatId", i);
      const tgMsgId = this.getNodeParameter("tgMsgId", i);
      const content = this.getNodeParameter("content", i);
      const senderId = this.getNodeParameter("senderId", i);
      const clusterId = this.getNodeParameter("clusterId", i);
      const status = this.getNodeParameter("status", i);
      const limit = this.getNodeParameter("limit", i);
      const windowMinutes = this.getNodeParameter("windowMinutes", i);
      const dryRun = this.getNodeParameter("dryRun", i);
      const banSenders = this.getNodeParameter("banSenders", i);

      let url = baseUrl + "/api/agent/tools/query-ad-clusters";
      let body = {
        tenant_id: tenantId || undefined,
        group_id: chatId ? Number(chatId) : undefined,
        status,
        limit,
        window_minutes: windowMinutes,
      };

      if (operation === "ingest") {
        url = baseUrl + "/api/ad-pipeline/events/group-message";
        body = {
          tenant_id: tenantId || undefined,
          chat_id: chatId ? Number(chatId) : undefined,
          group_id: chatId ? Number(chatId) : undefined,
          tg_msg_id: tgMsgId ? Number(tgMsgId) : undefined,
          content,
          sender_user_id: senderId ? Number(senderId) : undefined,
          sender_username: items[i].json.sender_username || items[i].json.sender_name || undefined,
          sender_type: items[i].json.sender_type || undefined,
          message_ts: items[i].json.message_ts || undefined,
          event: items[i].json.event || "group_message",
        };
      } else if (operation === "backfill") {
        url = baseUrl + "/api/ad-pipeline/backfill";
        body = { tenant_id: tenantId || undefined, days: items[i].json.days || 3, limit };
      } else if (operation === "delete") {
        url = baseUrl + "/api/agent/tools/delete-ad-cluster";
        body = {
          tenant_id: tenantId || undefined,
          cluster_id: clusterId,
          message_refs: items[i].json.message_refs || undefined,
          dry_run: dryRun,
          ban_senders: banSenders,
        };
      }

      const response = await this.makeRequest({
        url,
        method: "POST",
        body,
        headers: { "Authorization": "Bearer " + apiKey, "Content-Type": "application/json" },
      });

      items[i].json = {
        success: response.status === "ok" || response.success === true,
        operation,
        data: response.data || response,
        count: response.count,
        evidence_count: response.evidence_count,
        analysis_allowed: response.analysis_allowed,
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
