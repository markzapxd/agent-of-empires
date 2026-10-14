//! Backfill repository/workspace identity for retained agent graph observations.

use anyhow::Result;

pub fn run() -> Result<()> {
    let app_dir = crate::session::get_app_dir()?;
    let profiles = app_dir.join("profiles");
    if !profiles.exists() {
        return Ok(());
    }
    for entry in std::fs::read_dir(profiles)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            crate::observability::backfill_project_keys(&entry.path())?;
        }
    }
    Ok(())
}
