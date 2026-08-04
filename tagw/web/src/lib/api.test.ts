import { describe, expect, it } from 'vitest';
import {
  API_KEY_PROVIDER_TYPES,
  exportDbUrl,
  OAUTH_PROVIDERS,
  oauthStartUrl,
  rangeQuery,
  RANGES,
  type Range,
} from './api';

describe('rangeQuery', () => {
  it('encodes each supported range', () => {
    for (const range of RANGES) {
      expect(rangeQuery(range)).toBe(`range=${range}`);
    }
  });

  it('url-encodes special characters if present', () => {
    // Range is a closed union, but the helper should still encode.
    const weird = '7d' as Range;
    expect(rangeQuery(weird)).toBe('range=7d');
  });
});

describe('admin helpers', () => {
  it('lists oauth and api_key provider types', () => {
    expect(OAUTH_PROVIDERS).toContain('codex');
    expect(OAUTH_PROVIDERS).toContain('claude');
    expect(OAUTH_PROVIDERS).toContain('xai');
    expect(OAUTH_PROVIDERS).toContain('kimi');
    expect(OAUTH_PROVIDERS).toContain('antigravity');
    expect(API_KEY_PROVIDER_TYPES).toContain('glm');
    expect(API_KEY_PROVIDER_TYPES).toContain('openai_compat');
  });

  it('builds oauth start and db export urls', () => {
    expect(oauthStartUrl('codex')).toBe('/api/oauth/codex/start');
    expect(oauthStartUrl('xai')).toBe('/api/oauth/xai/start');
    expect(exportDbUrl()).toBe('/api/admin/export/db');
  });
});
