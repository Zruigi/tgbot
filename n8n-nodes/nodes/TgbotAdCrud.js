/**
 * TGBot Advertisement CRUD Node
 */
const http = require("http");
const https = require("https");

module.exports = {
  displayName: "TGBot Ad Management",
  name: "TgbotAdCrud",
  group: ["transform"],
  version: 1,
  description: "Advertisement register, query, delete operations",
  defaults: { name: "TGBot Ad Management", color: "#00cc66" },
  inputs: ["main"],
  outputs: ["main"],
  credentials: [{ name: "tgbotApi", required: true }],
  properties: [
    {
      displayName: "Operation",
      name: "operation",
      type: "options",
      options: [
        { name: "Register (Create)", value: "create" },
        { name: "Query (Read)", value: "read" },
        { name: "Delete", value: "delete" },
        { name: "List All", value: "list" },
      ],
      default: "create",
    },
    {
      displayName: "Ad ID",
      name: "adId",
      type: "string",
      default: "",
      placeholder: "Advertisement ID (for read/delete)",
    },
    {
      displayName: "Content",
      name: "content",
      type: "string",
      typeOptions: { rows: 3 },
      default: "",
      placeholder: "Ad content to register",
    },
    {
      displayName: "Score",
      name: "score",
      type: "number",
      default: 0,
    },
    {
      displayName: "Source",
      name: "source",
      type: "string",
      default: "manual",
    },
  ],
  async execute() {
    const items = this.getInputData();
    const creds = await this.getCredentials("tgbotApi");
    const { baseUrl, apiKey } = creds;
    
    for (let i = 0; i < items.length; i++) {
      const operation = this.getNodeParameter("operation", i);
      const adId = this.getNodeParameter("adId", i);
      const content = this.getNodeParameter("content", i);
      const score = this.getNodeParameter("score", i);
      const source = this.getNodeParameter("source", i);
      
      let url = baseUrl + "/api/agent/ad-fingerprints";
      let method = "POST";
      let body = { content, score, confirmed_by: source };
      
      if (operation === "read" && adId) {
        url = baseUrl + "/api/agent/ad-fingerprints?limit=200";
        method = "GET";
        body = null;
      } else if (operation === "delete" && adId) {
        url = baseUrl + "/api/agent/ad-fingerprints/" + encodeURIComponent(adId);
        method = "DELETE";
        body = null;
      } else if (operation === "list") {
        url = baseUrl + "/api/agent/ad-fingerprints?limit=200";
        method = "GET";
        body = null;
      }
      
      const response = await this.makeRequest({
        url: url, method: method, body: body,
        headers: { "Authorization": "Bearer " + apiKey, "Content-Type": "application/json" },
      });
      
      const rows = response.data || [];
      items[i].json = {
        success: response.status === "ok",
        operation,
        data: operation === "read" ? rows.find(row => row.text_hash === adId) || null : response,
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
