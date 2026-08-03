import { describe, expect, it } from 'vitest';
import { rangeQuery, RANGES, type Range } from './api';

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
