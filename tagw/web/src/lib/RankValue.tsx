import type { ReactNode } from 'react';
import {
  formatMs,
  formatNumber,
  rankClass,
  rankInputTokens,
  rankLatencyMs,
  rankOutputTokens,
  rankTtftMs,
  type Rank,
} from './format';

export function RankValue({
  rank,
  children,
  title,
}: {
  rank: Rank;
  children: ReactNode;
  title?: string;
}) {
  return (
    <span className={`rank ${rankClass(rank)}`} title={title}>
      {rank !== 'neutral' ? <span className="rank-dot" aria-hidden /> : null}
      {children}
    </span>
  );
}

export function RankedInputTokens({ n }: { n: number }) {
  const rank = rankInputTokens(n);
  return (
    <RankValue rank={rank} title={`Input tokens · ${rankLabel(rank)}`}>
      {formatNumber(n)}
    </RankValue>
  );
}

export function RankedOutputTokens({ n }: { n: number }) {
  const rank = rankOutputTokens(n);
  return (
    <RankValue rank={rank} title={`Output tokens · ${rankLabel(rank)}`}>
      {formatNumber(n)}
    </RankValue>
  );
}

export function RankedLatency({ ms }: { ms: number | null | undefined }) {
  if (ms == null) return <span className="muted">—</span>;
  const rank = rankLatencyMs(ms);
  return (
    <RankValue rank={rank} title={`Total latency · ${rankLabel(rank)}`}>
      {formatMs(ms)}
    </RankValue>
  );
}

export function RankedTtft({ ms }: { ms: number | null | undefined }) {
  if (ms == null) return <span className="muted">—</span>;
  const rank = rankTtftMs(ms);
  return (
    <RankValue rank={rank} title={`Time to first token · ${rankLabel(rank)}`}>
      {formatMs(ms)}
    </RankValue>
  );
}

export function TokensInOut({
  input,
  output,
  incomplete,
}: {
  input: number;
  output: number;
  incomplete?: boolean;
}) {
  return (
    <span className="tokens-split">
      <RankedInputTokens n={input} />
      <span className="sep">/</span>
      <RankedOutputTokens n={output} />
      {incomplete ? <span className="muted">*</span> : null}
    </span>
  );
}

export function RankLegend() {
  return (
    <div className="legend-row" aria-label="Color legend">
      <span>
        <span className="rank rank-good">
          <span className="rank-dot" /> good
        </span>{' '}
        light / fast
      </span>
      <span>
        <span className="rank rank-warn">
          <span className="rank-dot" /> warn
        </span>{' '}
        elevated
      </span>
      <span>
        <span className="rank rank-bad">
          <span className="rank-dot" /> heavy
        </span>{' '}
        slow / large
      </span>
    </div>
  );
}

function rankLabel(rank: Rank): string {
  switch (rank) {
    case 'good':
      return 'good (blue)';
    case 'warn':
      return 'elevated (yellow)';
    case 'bad':
      return 'heavy/slow (red)';
    default:
      return 'n/a';
  }
}
