import { describe, expect, it } from "vitest";

import REGISTRY from "../../open-sse/providers/registry/index.js";
import { PROVIDERS, PROVIDER_MODELS } from "../../open-sse/providers/index.js";
import { getModelTargetFormat, PROVIDER_ID_TO_ALIAS } from "../../open-sse/config/providerModels.js";
import { resolveTransport, getTargetFormat } from "../../open-sse/services/provider.js";
import { FILTERS } from "../../src/app/api/providers/suggested-models/filters.js";

describe("OpenModel provider", () => {
  const entry = REGISTRY.find((e) => e.id === "openmodel");

  it("is registered as a multi-protocol apikey provider", () => {
    expect(entry).toBeDefined();
    expect(entry.category).toBe("apikey");
    expect(entry.alias).toBe("openmodel");
    expect(entry.aliases).toContain("om");
    expect(entry.transport.format).toBe("claude");
    expect(entry.transport.baseUrl).toBe("https://api.openmodel.ai/v1/messages");
    expect(entry.transport.validateUrl).toBe("https://api.openmodel.ai/v1/models");
  });

  it("exposes Messages / Responses / Gemini transports with correct auth", () => {
    const byFormat = Object.fromEntries(entry.transports.map((t) => [t.format, t]));
    expect(byFormat.claude.baseUrl).toBe("https://api.openmodel.ai/v1/messages");
    expect(byFormat.claude.auth.header).toBe("x-api-key");
    expect(byFormat["openai-responses"].baseUrl).toBe("https://api.openmodel.ai/v1/responses");
    expect(byFormat["openai-responses"].auth.scheme).toBe("bearer");
    expect(byFormat.gemini.baseUrl).toBe("https://api.openmodel.ai/v1beta/models");
    expect(byFormat.gemini.auth.header).toBe("x-goog-api-key");
  });

  it("builds into the runtime PROVIDERS map", () => {
    expect(PROVIDERS.openmodel).toBeDefined();
    expect(PROVIDERS.openmodel.format).toBe("claude");
    expect(PROVIDERS.openmodel.baseUrl).toBe("https://api.openmodel.ai/v1/messages");
    expect(PROVIDERS.openmodel.transports).toHaveLength(3);
    expect(getTargetFormat("openmodel")).toBe("claude");
  });

  it("seeds Messages, Responses, and Gemini models with the right targetFormat", () => {
    const models = PROVIDER_MODELS.openmodel || [];
    const ids = models.map((m) => m.id);
    expect(ids).toContain("claude-sonnet-4-6");
    expect(ids).toContain("gpt-5.5");
    expect(ids).toContain("gemini-3.5-flash");
    expect(ids).toContain("deepseek-v4-flash");
    expect(ids).toContain("qwen3.7-max");

    expect(getModelTargetFormat("openmodel", "claude-sonnet-4-6")).toBeNull();
    expect(getModelTargetFormat("openmodel", "gpt-5.5")).toBe("openai-responses");
    expect(getModelTargetFormat("openmodel", "gemini-3.5-flash")).toBe("gemini");
    expect(getModelTargetFormat("om", "gpt-5.4-mini")).toBe("openai-responses");
  });

  it("infers protocol for passthrough model ids not in the seed list", () => {
    expect(getModelTargetFormat("openmodel", "gpt-99-future")).toBe("openai-responses");
    expect(getModelTargetFormat("openmodel", "gemini-9-pro")).toBe("gemini");
    expect(getModelTargetFormat("openmodel", "some-new-claude-like")).toBeNull();
  });

  it("resolves runtime transport from model targetFormat", () => {
    const responses = resolveTransport("openmodel", "openai-responses");
    expect(responses?.baseUrl).toBe("https://api.openmodel.ai/v1/responses");

    const gemini = resolveTransport("openmodel", "gemini");
    expect(gemini?.baseUrl).toBe("https://api.openmodel.ai/v1beta/models");

    const claude = resolveTransport("openmodel", "claude");
    expect(claude?.baseUrl).toBe("https://api.openmodel.ai/v1/messages");
  });

  it("maps alias openmodel/om to provider id", () => {
    expect(PROVIDER_ID_TO_ALIAS.openmodel).toBe("openmodel");
  });

  it("enables dynamic model discovery via public catalog", () => {
    expect(entry.passthroughModels).toBe(true);
    expect(entry.modelsFetcher).toMatchObject({
      url: "https://api.openmodel.ai/web/v1/models?pageSize=100",
      type: "openmodel",
    });
  });

  it("filters OpenModel public catalog shape into id/name pairs", () => {
    const raw = [
      { key: "claude-sonnet-4-6", provider_name: "Anthropic" },
      { key: "gpt-5.5", provider_name: "OpenAI" },
      { id: "legacy-id-only" },
      {},
    ];
    expect(FILTERS.openmodel(raw)).toEqual([
      { id: "claude-sonnet-4-6", name: "claude-sonnet-4-6" },
      { id: "gpt-5.5", name: "gpt-5.5" },
      { id: "legacy-id-only", name: "legacy-id-only" },
    ]);
  });

  it("keeps every registry id unique after adding openmodel", () => {
    const ids = REGISTRY.map((e) => e.id);
    expect(new Set(ids).size).toBe(ids.length);
  });
});
