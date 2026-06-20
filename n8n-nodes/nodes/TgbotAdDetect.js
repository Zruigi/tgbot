/**
 * TGBot Advertisement Detection Node
 */
const https = require("https");

module.exports = {
  displayName: "TGBot Ad Detection",
  name: "TgbotAdDetect",
  group: ["transform"],
  version: 1,
  description: "Detect advertisement in text",
  defaults: { name: "TGBot Ad Detection", color: "#ff6600" },
  inputs: ["main"],
  outputs: ["main"],
  credentials: [{ name: "tgbotApi", required: true }],
  properties: [
    {
      displayName: "Text",
      name: "text",
      type: "string",
      typeOptions: { rows: 3 },
      default: "={{ $json.text }}",
      required: true,
    },
    {
      displayName: "Context Type",
      name: "contextType",
      type: "options",
      options: [
        { name: "Group Message", value: "group_message" },
        { name: "Private Message", value: "private_message" },
        { name: "Channel Post", value: "channel" },
      ],
      default: "group_message",
    },
  ],
  async execute(this) {
    const items = this.getInputData();
    const creds = await this.getCredentials("tgbotApi");
    const { baseUrl, apiKey } = creds;
    
    for (let i = 0; i < items.length; i++) {
      const text = this.getNodeParameter("text", i);
      const contextType = this.getNodeParameter("contextType", i);
      
      const response = await this.makeRequest({
        url: baseUrl + "/api/ai/detect_ad",
        method: "POST",
        body: { text: text, context_type: contextType },
        headers: { "Authorization": "Bearer " + apiKey, "Content-Type": "application/json" },
      });
      
      items[i].json = {
        is_advertising: response.is_advertising || false,
        ad_score: response.ad_score || 0,
        confidence: response.confidence || "low",
        risk_level: response.risk_level || "low",
        findings: response.findings || [],
        recommendation: response.recommendation || "",
      };
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
