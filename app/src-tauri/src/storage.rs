use jitmo_store::Store;

/// The one place the app opens its database: encrypted with the key from
/// Credential Manager. A plain database from an earlier version is converted
/// (and backed up) the first time.
pub(crate) fn open_store(
    db: &std::path::Path,
    key: Option<&jitmo_store::Key>,
) -> Result<Store, jitmo_store::StoreError> {
    match key {
        Some(key) => Store::open_encrypted_migrating(db, key),
        // Development builds (no `encrypted-db` feature) keep a plain file.
        None => Store::open(db, None),
    }
}

/// Read (or create) the database key. There is deliberately no fallback to an
/// unencrypted file: if the key cannot be had, the app does not start.
pub(crate) fn database_key() -> Result<Option<jitmo_store::Key>, String> {
    #[cfg(not(feature = "encrypted-db"))]
    {
        Ok(None)
    }
    #[cfg(all(feature = "encrypted-db", windows))]
    {
        jitmo_adapter_windows::credentials::get_or_create_db_key("jitmo").map(Some)
    }
    #[cfg(all(feature = "encrypted-db", not(windows)))]
    {
        Err("the database key is kept in Windows Credential Manager".to_string())
    }
}
