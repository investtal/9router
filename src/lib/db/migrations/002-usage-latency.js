// Additive: capture per-request latency so TPS (mean/p50/p95/throughput) is
// computable per apiKey x model. No data is destroyed; existing rows backfill to 0.
//
// Columns are also declared in TABLES.usageHistory so syncSchemaFromTables
// backfills them. Guard the ALTERs so fresh DBs (where 001 already created the
// columns via buildCreateTableSql) don't hit "duplicate column name".

function hasColumn(db, table, col) {
  const rows = db.all(`PRAGMA table_info(${table})`);
  return Array.isArray(rows) && rows.some((r) => r.name === col);
}

export default {
  version: 2,
  name: "usage-latency",
  up(db) {
    if (!hasColumn(db, "usageHistory", "latencyTotalMs")) {
      db.exec(`ALTER TABLE usageHistory ADD COLUMN latencyTotalMs INTEGER DEFAULT 0`);
    }
    if (!hasColumn(db, "usageHistory", "latencyTtftMs")) {
      db.exec(`ALTER TABLE usageHistory ADD COLUMN latencyTtftMs INTEGER DEFAULT 0`);
    }
    db.exec(`CREATE INDEX IF NOT EXISTS idx_uh_key_model_ts ON usageHistory(apiKey, model, timestamp DESC)`);
  },
};
