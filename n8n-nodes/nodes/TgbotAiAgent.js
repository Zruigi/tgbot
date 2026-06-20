/**
 * TGBot AI Agent Node for n8n
 */
const https = require("https");

module.exports = {
  displayName: "TGBot AI Agent",
  name: "TgbotAiAgent",
  icon: "file:tgbot.svg",
  group: ["ai"],
  version: 1,
  description: "AI Agent for tgbot with function calling",
  defaults: { name: "TGBot AI Agent", color: "#0088cc" },
  inputs: ["main"],
  outputs: ["main"],
  credentials: [{ name: "tgbotApi", required: true }],
  properties: [
    {
      displayName: "Prompt",
      name: "prompt",
      type: "string",
      typeOptions: { rows: 4 },
      default: "",
      placeholder: "What do you want the AI to do?",
      required: true,
    },
    {
      displayName: "Model",
      name: "model",
      type: "string",
      default: "gemini-2.5-flash-lite",
    },
  ],
  async execute(this) {
    const items = this.getInputData();
    const creds = await this.getCredentials("tgbotApi");
    const { baseUrl, apiKey } = creds;
    
    for (let i = 0; i < items.length; i++) {
      const prompt = this.getNodeParameter("prompt", i);
      const model = this.getNodeParameter("model", i);
      
      const response = await this.makeRequest({
        url: baseUrl + "/api/ai/chat",
        method: "POST",
        body: { message: prompt, model: model },
        headers: { "Authorization": "Bearer " + apiKey, "Content-Type": "application/json" },
      });
      
      items[i].json = { success: true, response: response.text, tool_calls: response.tool_calls_made };
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
        res.on("end", () => { try { resolve(JSON.parse(data)); } catch { resolve({ text: data }); } });
      });
      req.on("error", reject);
      if (options.body) req.write(JSON.stringify(options.body));
      req.end();
    });
  },
};
