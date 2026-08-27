use proxi::{Context, ContextOptions, ProxiError, TransformerBuilder};
use std::path::{Path, PathBuf};

fn copy_database(name: &str, source_dir: &Path) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "proxi-context-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock")
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).expect("create test data directory");
    std::fs::copy(source_dir.join("proj.db"), root.join("proj.db")).expect("copy proj.db");
    root
}

fn installed_database_dir() -> PathBuf {
    Context::new()
        .expect("default context")
        .data_dir()
        .expect("default context must report the active PROJ data directory")
}

#[test]
fn explicit_database_path_wins_over_search_paths() {
    let source = installed_database_dir();
    let first = copy_database("first", &source);
    let second = copy_database("second", &source);
    let options = ContextOptions::default()
        .database_path(first.join("proj.db"))
        .push_data_path(second.clone());
    let context = Context::configure(&options).expect("configure context");

    assert_eq!(context.data_dir(), Some(first.clone()));
    assert!(context.data_paths().search_paths.contains(&second));

    std::fs::remove_dir_all(first).ok();
    std::fs::remove_dir_all(second).ok();
}

#[test]
fn independent_contexts_use_their_own_databases() {
    let source = installed_database_dir();
    let first = copy_database("isolated-first", &source);
    let second = copy_database("isolated-second", &source);
    let first_options = ContextOptions::default().database_path(first.join("proj.db"));
    let second_options = ContextOptions::default().database_path(second.join("proj.db"));
    let first_context = Context::configure(&first_options).expect("first context");
    let second_context = Context::configure(&second_options).expect("second context");

    assert_eq!(first_context.data_dir(), Some(first.clone()));
    assert_eq!(second_context.data_dir(), Some(second.clone()));
    TransformerBuilder::new(&first_context, "EPSG:4326", "EPSG:3857")
        .build()
        .expect("first context transform");
    TransformerBuilder::new(&second_context, "EPSG:4326", "EPSG:3857")
        .build()
        .expect("second context transform");

    std::fs::remove_dir_all(first).ok();
    std::fs::remove_dir_all(second).ok();
}

#[test]
fn missing_explicit_database_is_rejected() {
    let missing = std::env::temp_dir().join(format!(
        "proxi-missing-{}-{}.db",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock")
            .as_nanos()
    ));
    let options = ContextOptions::default().database_path(missing);
    let error = match Context::configure(&options) {
        Ok(_) => panic!("missing database must fail"),
        Err(error) => error,
    };
    assert!(matches!(error, ProxiError::MissingData { .. }));
}
