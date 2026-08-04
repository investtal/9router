/** Shared display formatters for the TAGW dashboard. */

/** Vietnam-readable thousands grouping (example: 5,597,800). */
const VN_NUM = new Intl.NumberFormat('en-US', { maximumFractionDigits: 0 });

/** Cost with up to 6 decimal places, still grouped. */
const VN_COST = new Intl.NumberFormat('en-US', {
  minimumFractionDigits: 2,
  maximumFractionDigits: 6,
});

export function formatNumber(n: number | null | undefined): string {
  if (n == null || Number.isNaN(n)) return '—';
  return VN_NUM.format(Math.round(n));
}

export function formatCost(n: number | null | undefined): string {
  if (n == null || Number.isNaN(n)) return '—';
  return `$${VN_COST.format(n)}`;
}

/**
 * Compact wall time: `06 Jul 2026 14:45:01` (UTC-offset preserved from input).
 */
export function formatDateTime(iso: string | null | undefined): string {
  if (!iso) return '—';
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  const months = [
    'Jan',
    'Feb',
    'Mar',
    'Apr',
    'May',
    'Jun',
    'Jul',
    'Aug',
    'Sep',
    'Oct',
    'Nov',
    'Dec',
  ];
  const dd = String(d.getDate()).padStart(2, '0');
  const mon = months[d.getMonth()];
  const yyyy = d.getFullYear();
  const hh = String(d.getHours()).padStart(2, '0');
  const mm = String(d.getMinutes()).padStart(2, '0');
  const ss = String(d.getSeconds()).padStart(2, '0');
  return `${dd} ${mon} ${yyyy} ${hh}:${mm}:${ss}`;
}

export type Rank = 'good' | 'warn' | 'bad' | 'neutral';

/**
 * Traffic-light rank for a metric.
 * - good (blue): healthy / light
 * - warn (yellow): elevated
 * - bad (red): heavy / slow
 */
export function rankLatencyMs(ms: number | null | undefined): Rank {
  if (ms == null) return 'neutral';
  if (ms < 2_000) return 'good';
  if (ms < 8_000) return 'warn';
  return 'bad';
}

export function rankTtftMs(ms: number | null | undefined): Rank {
  if (ms == null) return 'neutral';
  if (ms < 800) return 'good';
  if (ms < 2_500) return 'warn';
  return 'bad';
}

/** Input/prompt tokens — high context cost. */
export function rankInputTokens(n: number | null | undefined): Rank {
  if (n == null) return 'neutral';
  if (n < 8_000) return 'good';
  if (n < 40_000) return 'warn';
  return 'bad';
}

/** Output/completion tokens. */
export function rankOutputTokens(n: number | null | undefined): Rank {
  if (n == null) return 'neutral';
  if (n < 1_500) return 'good';
  if (n < 6_000) return 'warn';
  return 'bad';
}

export function rankClass(rank: Rank): string {
  switch (rank) {
    case 'good':
      return 'rank-good';
    case 'warn':
      return 'rank-warn';
    case 'bad':
      return 'rank-bad';
    default:
      return 'rank-neutral';
  }
}

export function formatMs(ms: number | null | undefined): string {
  if (ms == null) return '—';
  return `${formatNumber(ms)} ms`;
}

/** Extract provider prefix from `glm/glm-5.2` → `glm`. */
export function providerFromModel(model: string | null | undefined): string | null {
  if (!model) return null;
  const i = model.indexOf('/');
  if (i <= 0) return null;
  return model.slice(0, i).toLowerCase();
}
