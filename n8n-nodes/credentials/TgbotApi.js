/**
 * TGBot API Credentials for n8n
 */
module.exports = {
  name: "tgbotApi",
  displayName: "TGBot API",
  type: "credentialType",
  properties: [
    {
      displayName: "Base URL",
      name: "baseUrl",
      type: "string",
      default: "https://your-tgbot-domain.com",
      placeholder: "https://your-tgbot-domain.com",
      required: true,
    },
    {
      displayName: "API Key",
      name: "apiKey",
      type: "string",
      typeOptions: { password: true },
      default: "",
      required: true,
    },
  ],
};
