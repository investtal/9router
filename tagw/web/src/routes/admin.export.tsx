import { createFileRoute } from '@tanstack/react-router';
import { useState, type FormEvent } from 'react';
import {
  exportBundle,
  exportDbUrl,
  importBundle,
  type ExportBundle,
  type ImportResult,
} from '../lib/api';

export const Route = createFileRoute('/admin/export')({
  component: AdminExportPage,
});

function AdminExportPage() {
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [bundleJson, setBundleJson] = useState('');
  const [importText, setImportText] = useState('');
  const [importResult, setImportResult] = useState<ImportResult | null>(null);
  const [exportedMeta, setExportedMeta] = useState<string | null>(null);

  async function onExportBundle() {
    setBusy(true);
    setError(null);
    setImportResult(null);
    try {
      const bundle = await exportBundle();
      const text = JSON.stringify(bundle, null, 2);
      setBundleJson(text);
      setExportedMeta(
        `version ${bundle.version} · ${bundle.exported_at} · ${bundle.providers.length} providers · ${bundle.accounts.length} accounts`,
      );
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  }

  function onDownloadBundle() {
    if (!bundleJson) return;
    const blob = new Blob([bundleJson], { type: 'application/json' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = `tagw-bundle-${new Date().toISOString().slice(0, 10)}.json`;
    a.click();
    URL.revokeObjectURL(url);
  }

  async function onImport(e: FormEvent) {
    e.preventDefault();
    setBusy(true);
    setError(null);
    setImportResult(null);
    try {
      const parsed = JSON.parse(importText) as ExportBundle;
      const result = await importBundle(parsed);
      setImportResult(result);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div>
      <h1>Admin · export / import</h1>
      <p className="muted">
        Download a portable JSON bundle (includes secrets for restore) or the raw SQLite DB.
        Import replaces/merges according to the server bundle import rules.
      </p>
      {error ? <div className="error card">{error}</div> : null}

      <div className="card">
        <strong>Export</strong>
        <div className="row" style={{ marginTop: '0.75rem' }}>
          <button type="button" disabled={busy} onClick={() => void onExportBundle()}>
            Export JSON bundle
          </button>
          <button
            type="button"
            className="secondary"
            disabled={!bundleJson}
            onClick={onDownloadBundle}
          >
            Download .json
          </button>
          <a className="btn-link" href={exportDbUrl()}>
            Download SQLite DB
          </a>
        </div>
        {exportedMeta ? (
          <p className="muted" style={{ marginTop: '0.5rem' }}>
            {exportedMeta}
          </p>
        ) : null}
        {bundleJson ? (
          <textarea
            className="mono"
            readOnly
            value={bundleJson}
            rows={12}
            style={{ width: '100%', marginTop: '0.75rem' }}
          />
        ) : null}
      </div>

      <form className="card" onSubmit={onImport}>
        <strong>Import JSON bundle</strong>
        <p className="muted" style={{ margin: '0.35rem 0 0.75rem' }}>
          Paste a previously exported bundle. Requires admin role.
        </p>
        <textarea
          className="mono"
          value={importText}
          onChange={(e) => setImportText(e.target.value)}
          rows={10}
          placeholder='{"version":1,...}'
          style={{ width: '100%' }}
          required
        />
        <div className="row" style={{ marginTop: '0.75rem' }}>
          <button type="submit" disabled={busy || !importText.trim()}>
            Import bundle
          </button>
        </div>
        {importResult ? (
          <pre className="mono" style={{ marginTop: '0.75rem', fontSize: '0.85rem' }}>
            {JSON.stringify(importResult, null, 2)}
          </pre>
        ) : null}
      </form>
    </div>
  );
}
