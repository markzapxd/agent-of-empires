//! Seed durable profile-local graph projections from retained sessions.

use anyhow::{Context, Result};
use std::{fs, path::Path};

pub fn run() -> Result<()> {
    run_in(&crate::session::get_app_dir()?)
}

fn run_in(app_dir: &Path) -> Result<()> {
    let profiles = app_dir.join("profiles");
    if !profiles.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(profiles)? {
        let entry = entry?;
        let dir = entry.path();
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let Some(profile) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let path = dir.join("sessions.json");
        let _lock =
            crate::session::acquire_storage_flock(&dir, crate::session::STORAGE_LOCK_FILENAME)?;
        let mut rows: Vec<crate::session::Instance> = match fs::read(&path) {
            Ok(bytes) => {
                let values: Vec<serde_json::Value> = serde_json::from_slice(&bytes)
                    .with_context(|| format!("reading {}", path.display()))?;
                values
                    .into_iter()
                    .filter_map(|value| match serde_json::from_value(value) {
                        Ok(row) => Some(row),
                        Err(error) => {
                            tracing::warn!(path = %path.display(), "skipping corrupt session row during graph backfill: {error}");
                            None
                        }
                    })
                    .collect()
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(error) => return Err(error.into()),
        };
        for row in &mut rows {
            row.source_profile.clone_from(&profile);
        }
        crate::observability::observe_sessions(
            &profile,
            &dir,
            &rows,
            crate::observability::ObserveOptions {
                mark_missing_unavailable: true,
            },
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::Instance;

    #[test]
    fn backfills_profiles_without_modifying_sessions_and_is_idempotent() -> Result<()> {
        let app = tempfile::tempdir()?;
        let profile_dir = app.path().join("profiles/default");
        fs::create_dir_all(&profile_dir)?;
        let mut parent = Instance::new("ancestor", "/repo");
        let mut child = Instance::new("descendant", "/repo");
        child.parent_session_id = Some(parent.id.clone());
        parent.source_profile = "default".into();
        let sessions = serde_json::to_vec(&[parent, child])?;
        fs::write(profile_dir.join("sessions.json"), &sessions)?;
        run_in(app.path())?;
        let first = crate::observability::snapshot(&profile_dir)?;
        assert_eq!(first.nodes.len(), 2);
        assert_eq!(first.edges.len(), 1);
        assert!(first.nodes.iter().all(|node| node.available));
        run_in(app.path())?;
        assert_eq!(
            crate::observability::snapshot(&profile_dir)?.cursor,
            first.cursor
        );
        assert_eq!(fs::read(profile_dir.join("sessions.json"))?, sessions);
        Ok(())
    }
}
