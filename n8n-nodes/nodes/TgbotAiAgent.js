/**
 * TGBot AI Agent Node for n8n
 */
const http = require("http");
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
    {
      displayName: "User ID",
      name: "userId",
      type: "string",
      default: "-99999",
      placeholder: "-99999",
      description: "Conversation user id. Use -99999 for the virtual Agent assistant.",
    },
    {
      displayName: "Feature Key",
      name: "featureKey",
      type: "string",
      default: "n8n_agent",
    },
    {
      displayName: "Tenant ID",
      name: "tenantId",
      type: "string",
      default: "",
      description: "Optional tenant scope passed to the Agent context.",
    },
    {
      displayName: "Chat ID",
      name: "chatId",
      type: "string",
      default: "",
      description: "Optional Telegram chat_id/group_id context.",
    },
    {
      displayName: "Timeout Seconds",
      name: "timeoutSeconds",
      type: "number",
      default: 120,
      description: "Maximum time to wait for the Agent API before returning a timeout result.",
    },
    {
      displayName: "Fail Workflow On Error",
      name: "failWorkflowOnError",
      type: "boolean",
      default: false,
      description: "When disabled, API errors are returned as item JSON so downstream fallback nodes can run.",
    },
  ],
  async execute() {
    const items = this.getInputData();
    const creds = await this.getCredentials("tgbotApi");
    const { baseUrl, apiKey } = creds;
    
    for (let i = 0; i < items.length; i++) {
      const prompt = this.getNodeParameter("prompt", i);
      const model = this.getNodeParameter("model", i);
      const userId = this.getNodeParameter("userId", i);
      const featureKey = this.getNodeParameter("featureKey", i);
      const tenantId = this.getNodeParameter("tenantId", i);
      const chatId = this.getNodeParameter("chatId", i);
      const timeoutSeconds = Number(this.getNodeParameter("timeoutSeconds", i) || 120);
      const failWorkflowOnError = Boolean(this.getNodeParameter("failWorkflowOnError", i));
      
      try {
        const response = await this.makeRequest({
          url: baseUrl + "/api/ai/chat",
          method: "POST",
          timeoutMs: Math.max(1, timeoutSeconds) * 1000,
          body: {
            userId: Number(userId || -99999),
            content: prompt,
            message: prompt,
            model: model,
            feature_key: featureKey,
            tenant_id: tenantId || undefined,
            context: {
              tenant_id: tenantId || undefined,
              chat_id: chatId ? Number(chatId) : undefined,
              group_id: chatId ? Number(chatId) : undefined,
            },
          },
          headers: { "Authorization": "Bearer " + apiKey, "Content-Type": "application/json" },
        });
        
        items[i].json = {
          ...items[i].json,
          success: response.status === "ok",
          response: response.reply || (response.data && response.data.reply) || response.text || "",
          tool_calls: response.tool_calls_made || [],
          tool_results: response.tool_results || {},
          evidence: response.evidence || response.tool_results || {},
          data_complete: response.data_complete,
          analysis_allowed: response.analysis_allowed,
          requires_confirmation: Boolean(response.requires_confirmation),
          action_taken: Boolean(response.is_action_taken),
          raw: response,
        };
      } catch (error) {
        if (failWorkflowOnError) {
          throw error;
        }
        items[i].json = {
          ...items[i].json,
          success: false,
          response: "查询超时或工具执行失败，请稍后重试。",
          error: error.message || String(error),
          tool_calls: [],
          tool_results: {},
          evidence: {},
          data_complete: false,
          analysis_allowed: false,
          requires_confirmation: false,
          action_taken: false,
        };
      }
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
      const timeout = setTimeout(() => {
        req.destroy(new Error(`TGBot API timeout after ${options.timeoutMs}ms`));
      }, options.timeoutMs || 120000);
      req.on("error", reject);
      req.on("close", () => clearTimeout(timeout));
      if (options.body) req.write(JSON.stringify(options.body));
      req.end();
    });
  },
};
