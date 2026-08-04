use tagw::auth::member_key::{create_member_key, revoke_member_key};
use tagw::cache::ConfigCache;
use tagw::db::Db;

#[tokio::test]
async fn created_key_authenticates_and_revoked_does_not() {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path().join("gateway.db")).unwrap();
    db.migrate().unwrap();

    let (row, plaintext) = create_member_key(&db, "alice").unwrap();
    assert!(!plaintext.is_empty());
    assert!(plaintext.starts_with("sk-"));
    assert_eq!(row.name, "alice");
    assert_eq!(row.key_prefix.len(), 8);
    // Never store plaintext at rest — only prefix/hash on the row.
    assert_ne!(row.key_hash, plaintext);

    let cache = ConfigCache::new();
    cache.load(&db).unwrap();

    let ctx = cache
        .authenticate_bearer(&plaintext)
        .expect("created key should authenticate");
    assert_eq!(ctx.key_id, row.id);
    assert_eq!(ctx.name, "alice");

    // Wrong token must not authenticate.
    assert!(cache.authenticate_bearer("sk-not-a-real-key-zzzz").is_none());

    revoke_member_key(&db, &row.id).unwrap();
    cache.reload(&db).unwrap();

    assert!(
        cache.authenticate_bearer(&plaintext).is_none(),
        "revoked key must not authenticate"
    );
}
