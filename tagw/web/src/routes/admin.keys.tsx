import { createFileRoute } from '@tanstack/react-router';
import { useEffect, useState, type FormEvent } from 'react';
import {
  createAdminKey,
  fetchAdminKeys,
  revokeAdminKey,
  type CreateKeyResponse,
  type MemberApiKeyPublic,
} from '../lib/api';

export const Route = createFileRoute('/admin/keys')({
  component: AdminKeysPage,
});

function AdminKeysPage() {
  const [keys, setKeys] = useState<MemberApiKeyPublic[]>([]);
  const [name, setName] = useState('');
  const [error, setError] = useState<string | null>(null);
  const [created, setCreated] = useState<CreateKeyResponse | null>(null);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);

  async function reload() {
    setLoading(true);
    try {
      const rows = await fetchAdminKeys();
      setKeys(rows);
      setError(null);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    void reload();
  }, []);

  async function onCreate(e: FormEvent) {
    e.preventDefault();
    setBusy(true);
    setCreated(null);
    try {
      const res = await createAdminKey(name.trim());
      setCreated(res);
      setName('');
      await reload();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  }

  async function onRevoke(id: string) {
    if (!confirm('Revoke this member key?')) return;
    setBusy(true);
    try {
      await revokeAdminKey(id);
      await reload();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div>
      <h1>Admin · member keys</h1>
      <p className="muted">Requires admin role. Plaintext is shown only once on create.</p>
      {error ? <div className="error card">{error}</div> : null}
      {created ? (
        <div className="card">
          <strong>New key created — copy now</strong>
          <div className="mono" style={{ marginTop: 8, wordBreak: 'break-all' }}>
            {created.key}
          </div>
          <div className="muted" style={{ marginTop: 4 }}>
            prefix {created.key_prefix} · id {created.id}
          </div>
        </div>
      ) : null}
      <form className="card row" onSubmit={onCreate}>
        <label>
          Name{' '}
          <input
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder="alice-laptop"
            required
          />
        </label>
        <button type="submit" disabled={busy || !name.trim()}>
          Create key
        </button>
      </form>
      {loading ? <p className="muted">Loading…</p> : null}
      <div className="card" style={{ overflowX: 'auto' }}>
        <table>
          <thead>
            <tr>
              <th>Name</th>
              <th>Prefix</th>
              <th>Created</th>
              <th>Revoked</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {keys.map((k) => (
              <tr key={k.id}>
                <td>{k.name}</td>
                <td className="mono">{k.key_prefix}</td>
                <td className="mono">{k.created_at}</td>
                <td className="mono">{k.revoked_at ?? '—'}</td>
                <td>
                  {!k.revoked_at ? (
                    <button
                      type="button"
                      className="danger"
                      disabled={busy}
                      onClick={() => void onRevoke(k.id)}
                    >
                      Revoke
                    </button>
                  ) : null}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}
