#[tokio::test]
async fn migrate_creates_request_logs() {
    let dir = tempfile::tempdir().unwrap();
    let db = tagw::db::Db::open(dir.path().join("gateway.db")).unwrap();
    db.migrate().unwrap();
    let n: i64 = db
        .with_conn(|c| {
            c.query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name='request_logs'",
                [],
                |r| r.get(0),
            )
        })
        .unwrap();
    assert_eq!(n, 1);
}
