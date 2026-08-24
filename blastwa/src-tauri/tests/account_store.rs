// integration test: account identity store lifecycle
use blastwa_core::api::server;

#[test]
fn account_store_lifecycle() {
    let dir = std::env::temp_dir().join(format!("blastwa_store_test_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    // 1. empty when no file
    assert_eq!(server::load_saved_accounts(&dir), Vec::<String>::new());

    // 2. save + dedupe
    server::save_account_name(&dir, "oa").unwrap();
    server::save_account_name(&dir, "oa").unwrap();
    server::save_account_name(&dir, "akun2").unwrap();
    let saved = server::load_saved_accounts(&dir);
    assert_eq!(saved, vec!["oa".to_string(), "akun2".to_string()]);

    // 3. file contains identity only, never runtime state
    let raw = std::fs::read_to_string(server::accounts_file(&dir)).unwrap();
    assert!(!raw.contains("port"), "no port on disk");
    assert!(!raw.contains("connected"), "no connected flag on disk");

    // 4. remove
    server::remove_saved_account(&dir, "oa").unwrap();
    assert_eq!(server::load_saved_accounts(&dir), vec!["akun2".to_string()]);

    // 5. corrupt file -> safe empty fallback
    std::fs::write(server::accounts_file(&dir), "{corrupt").unwrap();
    assert_eq!(server::load_saved_accounts(&dir), Vec::<String>::new());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn account_store_survives_reopen() {
    // simulate app restart: same dir, fresh calls
    let dir = std::env::temp_dir().join(format!("blastwa_store_reopen_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    server::save_account_name(&dir, "persistent").unwrap();
    drop(server::load_saved_accounts(&dir)); // simulate process boundary

    let reopened = server::load_saved_accounts(&dir);
    assert_eq!(reopened, vec!["persistent".to_string()]);

    let _ = std::fs::remove_dir_all(&dir);
}
