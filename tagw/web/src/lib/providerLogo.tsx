import { useState } from 'react';

/** Map provider_type / model prefix → logo file under /providers/*.png */
const LOGO_ALIASES: Record<string, string> = {
  glm: 'glm',
  zai: 'glm',
  'z.ai': 'glm',
  xai: 'xai',
  grok: 'xai',
  codex: 'codex',
  openai: 'openai',
  openai_compat: 'openai',
  oai: 'openai',
  open_model: 'openai',
  anthropic: 'anthropic',
  claude: 'claude',
  antigravity: 'antigravity',
  ag: 'antigravity',
  google: 'antigravity',
  deepseek: 'deepseek',
  ds: 'deepseek',
  kimi: 'kimi',
  moonshot: 'kimi',
  minimax: 'minimax',
  mm: 'minimax',
  alibaba: 'qwen',
  qwen: 'qwen',
  dashscope: 'qwen',
};

const failed = new Set<string>();

function resolveLogoId(provider: string | null | undefined): string | null {
  if (!provider) return null;
  const key = provider.trim().toLowerCase();
  if (!key || failed.has(key)) return null;
  const id = LOGO_ALIASES[key] ?? key;
  if (failed.has(id)) return null;
  return id;
}

export function getProviderLogoSrc(provider: string | null | undefined): string | null {
  const id = resolveLogoId(provider);
  return id ? `/providers/${id}.png` : null;
}

export function ProviderLogo({
  provider,
  size = 20,
  className = '',
  title,
}: {
  provider: string | null | undefined;
  size?: number;
  className?: string;
  title?: string;
}) {
  const [broken, setBroken] = useState(false);
  const src = broken ? null : getProviderLogoSrc(provider);
  const label = (provider || '?').slice(0, 2).toUpperCase();

  if (!src) {
    return (
      <span
        className={`provider-logo-fallback ${className}`.trim()}
        style={{ width: size, height: size, fontSize: Math.max(9, size * 0.42) }}
        title={title || provider || undefined}
        aria-hidden
      >
        {label}
      </span>
    );
  }

  return (
    <img
      src={src}
      alt=""
      width={size}
      height={size}
      className={`provider-logo ${className}`.trim()}
      title={title || provider || undefined}
      onError={() => {
        const id = resolveLogoId(provider);
        if (id) failed.add(id);
        if (provider) failed.add(provider.toLowerCase());
        setBroken(true);
      }}
    />
  );
}

/** Logo + name inline chip. */
export function ProviderChip({
  provider,
  label,
  size = 18,
}: {
  provider: string | null | undefined;
  label?: string;
  size?: number;
}) {
  const text = label ?? provider ?? '—';
  return (
    <span className="provider-chip">
      <ProviderLogo provider={provider} size={size} />
      <span>{text}</span>
    </span>
  );
}
