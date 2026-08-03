import { createFileRoute } from '@tanstack/react-router';
import { useEffect, useRef, useState } from 'react';
import { fetchRecentLogs, type LiveEvent } from '../lib/api';

export const Route = createFileRoute('/logs')({
  component: LogsPage,
});

const MAX_LINES = 400;

function LogsPage() {
  const [events, setEvents] = useState<LiveEvent[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [connected, setConnected] = useState(false);
  const bottomRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    let cancelled = false;
    fetchRecentLogs(100)
      .then((rows) => {
        if (!cancelled) setEvents(rows);
      })
      .catch((err) => {
        if (!cancelled) setError(err instanceof Error ? err.message : String(err));
      });
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    const es = new EventSource('/api/logs/stream', { withCredentials: true });
    es.onopen = () => {
      setConnected(true);
      setError(null);
    };
    es.onerror = () => {
      setConnected(false);
      setError('SSE disconnected (will retry)');
    };
    es.onmessage = (msg) => {
      try {
        const ev = JSON.parse(msg.data) as LiveEvent;
        setEvents((prev) => {
          if (prev.some((p) => p.id === ev.id)) return prev;
          const next = [...prev, ev];
          if (next.length > MAX_LINES) return next.slice(next.length - MAX_LINES);
          return next;
        });
      } catch {
        // ignore bad payloads
      }
    };
    return () => {
      es.close();
    };
  }, []);

  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: 'smooth' });
  }, [events.length]);

  return (
    <div>
      <h1>Live logs</h1>
      <div className="row">
        <span className={`badge ${connected ? 'on' : 'off'}`}>
          {connected ? 'connected' : 'disconnected'}
        </span>
        <span className="muted">{events.length} events</span>
      </div>
      {error ? <div className="error card">{error}</div> : null}
      <div className="card" style={{ maxHeight: '70vh', overflow: 'auto' }}>
        {events.length === 0 ? <p className="muted">No events yet</p> : null}
        {events.map((ev) => (
          <div key={ev.id} className={`log-line ${ev.level}`}>
            <span className="muted">{ev.ts}</span> [{ev.level}] {ev.message}
            {ev.model ? ` model=${ev.model}` : ''}
            {ev.member_key_id ? ` member=${ev.member_key_id}` : ''}
          </div>
        ))}
        <div ref={bottomRef} />
      </div>
    </div>
  );
}
