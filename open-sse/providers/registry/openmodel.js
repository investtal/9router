import { CLAUDE_API_HEADERS } from "../shared.js";

const OM_BASE = "https://api.openmodel.ai";

const MESSAGES_AUTH = { combined: true, header: "x-api-key", scheme: "raw" };
const RESPONSES_AUTH = { combined: true, header: "Authorization", scheme: "bearer" };
const GEMINI_AUTH = { combined: true, header: "x-goog-api-key", scheme: "raw" };

export default {
  id: "openmodel",
  priority: 15,
  alias: "openmodel",
  aliases: ["om"],
  uiAlias: "om",
  display: {
    name: "OpenModel",
    icon: "hub",
    color: "#6366F1",
    textIcon: "OM",
    website: "https://www.openmodel.ai",
    notice: {
      text: "Unified multi-protocol gateway (Messages / Responses / Gemini). API keys start with om-. OpenAI models use Responses; Gemini uses the Gemini protocol; everything else uses Anthropic Messages.",
      apiKeyUrl: "https://console.openmodel.ai",
    },
  },
  category: "apikey",
  authType: "apikey",
  // Default to Messages — covers Anthropic, DeepSeek, DashScope, Xiaomi, Kimi, MiniMax, Zai.
  transport: {
    baseUrl: `${OM_BASE}/v1/messages`,
    validateUrl: `${OM_BASE}/v1/models`,
    format: "claude",
    headers: { ...CLAUDE_API_HEADERS },
    auth: MESSAGES_AUTH,
  },
  // Protocol endpoints. chatCore prefers model.targetFormat when resolving transport,
  // so OpenAI/Gemini models hit the correct baseUrl + auth even when the client is OpenAI chat.
  transports: [
    {
      format: "claude",
      baseUrl: `${OM_BASE}/v1/messages`,
      headers: { ...CLAUDE_API_HEADERS },
      auth: MESSAGES_AUTH,
    },
    {
      format: "openai-responses",
      baseUrl: `${OM_BASE}/v1/responses`,
      auth: RESPONSES_AUTH,
    },
    {
      format: "gemini",
      baseUrl: `${OM_BASE}/v1beta/models`,
      auth: GEMINI_AUTH,
    },
  ],
  models: [
    // Messages protocol (default)
    "claude-fable-5",
    "claude-haiku-4-5-20251001",
    "claude-opus-4-6",
    "claude-opus-4-7",
    "claude-opus-4-8",
    "claude-opus-5",
    "claude-sonnet-4-5",
    "claude-sonnet-4-6",
    "claude-sonnet-5",
    "deepseek-v4-flash",
    "deepseek-v4-pro",
    "glm-4.7",
    "glm-5",
    "glm-5.1",
    "glm-5.2",
    "kimi-k2.5",
    "kimi-k2.6",
    "kimi-k2.7-code",
    "kimi-k3",
    "mimo-v2-flash",
    "mimo-v2-omni",
    "mimo-v2-pro",
    "mimo-v2.5",
    "mimo-v2.5-pro",
    "qwen3-max",
    "qwen3.5-plus",
    "qwen3.6-flash",
    "qwen3.6-max-preview",
    "qwen3.6-plus",
    "qwen3.7-max",
    // Responses protocol (OpenAI / xAI / Tencent)
    { id: "gpt-5.3-codex", targetFormat: "openai-responses" },
    { id: "gpt-5.4", targetFormat: "openai-responses" },
    { id: "gpt-5.4-mini", targetFormat: "openai-responses" },
    { id: "gpt-5.4-pro", targetFormat: "openai-responses" },
    { id: "gpt-5.5", targetFormat: "openai-responses" },
    { id: "gpt-5.6-luna", targetFormat: "openai-responses" },
    { id: "gpt-5.6-sol", targetFormat: "openai-responses" },
    { id: "gpt-5.6-terra", targetFormat: "openai-responses" },
    { id: "grok-4.5", targetFormat: "openai-responses" },
    { id: "hy3", name: "Hunyuan 3", targetFormat: "openai-responses" },
    // Gemini protocol
    { id: "gemini-3-flash-preview", targetFormat: "gemini" },
    { id: "gemini-3.1-pro-preview", targetFormat: "gemini" },
    { id: "gemini-3.5-flash", targetFormat: "gemini" },
    { id: "gemini-3.5-flash-lite", targetFormat: "gemini" },
    { id: "gemini-3.6-flash", targetFormat: "gemini" },
  ],
  serviceKinds: ["llm", "imageToText"],
  // Public catalog — no auth required. Filter maps {key} → {id,name}.
  modelsFetcher: { url: `${OM_BASE}/web/v1/models?pageSize=100`, type: "openmodel" },
  passthroughModels: true,
};
