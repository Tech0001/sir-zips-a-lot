use std::{
    collections::{BTreeSet, HashMap},
    fs,
    io::Write,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};

use crate::{EntryState, archive_checked, is_link, snapshot};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    pub source: PathBuf,
    pub destination: PathBuf,
    pub quiet_seconds: u64,
}

impl Config {
    pub fn validate(&self) -> Result<Self> {
        ensure!(
            !self.source.as_os_str().is_empty(),
            "Choose an orders folder"
        );
        ensure!(
            !self.destination.as_os_str().is_empty(),
            "Choose a destination folder"
        );
        ensure!(
            (1..=3600).contains(&self.quiet_seconds),
            "Quiet period must be between 1 and 3600 seconds"
        );
        let source = self
            .source
            .canonicalize()
            .context("Cannot open the orders folder")?;
        ensure!(source.is_dir(), "Orders path must be a folder");
        fs::create_dir_all(&self.destination).context("Cannot create the destination folder")?;
        let destination = self
            .destination
            .canonicalize()
            .context("Cannot open the destination folder")?;
        ensure!(
            !destination.starts_with(&source) && !source.starts_with(&destination),
            "Choose separate orders and destination folders; neither may contain the other"
        );
        Ok(Self {
            source,
            destination,
            quiet_seconds: self.quiet_seconds,
        })
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Activity {
    pub kind: String,
    pub message: String,
}

struct Candidate {
    snapshot: Vec<EntryState>,
    unchanged_since: Instant,
}

#[derive(Default, Serialize, Deserialize)]
struct History {
    initialized_routes: BTreeSet<(PathBuf, PathBuf)>,
    completed: BTreeSet<(PathBuf, PathBuf)>,
}

/// Polling supports local folders and network shares without relying on OS events.
/// Completed names are remembered even after ZIPs leave the destination.
pub struct Watcher {
    config: Config,
    history_path: PathBuf,
    history: History,
    candidates: HashMap<PathBuf, Candidate>,
    last_errors: HashMap<PathBuf, String>,
}

impl Watcher {
    pub fn new(config: Config, history_path: PathBuf) -> Result<Self> {
        let config = config.validate()?;
        let mut history: History = match fs::read(&history_path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .context("Cannot read completion history; repair it before restarting")?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => History::default(),
            Err(error) => return Err(error).context("Cannot open completion history"),
        };
        let route = (config.source.clone(), config.destination.clone());
        if !history.initialized_routes.contains(&route) {
            // Establish the starting point once per route. On subsequent starts,
            // new orders that arrived while the app was closed are still picked up.
            for entry in fs::read_dir(&config.source).context("Cannot scan the orders folder")? {
                let entry = entry?;
                if entry.file_type()?.is_dir() {
                    history
                        .completed
                        .insert((entry.path(), config.destination.clone()));
                }
            }
            history.initialized_routes.insert(route);
            save_json(&history_path, &history).context("Cannot save the starting folder list")?;
        }
        Ok(Self {
            config,
            history_path,
            history,
            candidates: HashMap::new(),
            last_errors: HashMap::new(),
        })
    }

    pub fn pending(&self) -> usize {
        self.candidates.len()
    }

    pub fn poll(&mut self) -> Result<Vec<Activity>> {
        self.poll_while(|| true)
    }

    pub fn poll_while(&mut self, keep_running: impl FnMut() -> bool) -> Result<Vec<Activity>> {
        self.poll_at_while(Instant::now(), keep_running)
    }

    #[cfg(test)]
    fn poll_at(&mut self, now: Instant) -> Result<Vec<Activity>> {
        self.poll_at_while(now, || true)
    }

    fn poll_at_while(
        &mut self,
        now: Instant,
        mut keep_running: impl FnMut() -> bool,
    ) -> Result<Vec<Activity>> {
        let mut activity = Vec::new();
        let mut present = BTreeSet::new();
        let mut entries = fs::read_dir(&self.config.source)
            .context("Cannot scan the orders folder")?
            .collect::<std::io::Result<Vec<_>>>()?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            if !keep_running() {
                break;
            }
            let path = entry.path();
            let key = (path.clone(), self.config.destination.clone());
            if self.history.completed.contains(&key) {
                continue;
            }
            let metadata = match fs::symlink_metadata(&path) {
                Ok(metadata) => metadata,
                Err(error) => {
                    self.report_error(&path, error.to_string(), &mut activity);
                    continue;
                }
            };
            if !metadata.is_dir() && !is_link(&metadata) {
                continue;
            }
            present.insert(path.clone());
            let current = match snapshot(&path) {
                Ok(current) => current,
                Err(error) => {
                    self.candidates.remove(&path);
                    self.report_error(&path, format!("{error:#}"), &mut activity);
                    continue;
                }
            };
            let candidate = self
                .candidates
                .entry(path.clone())
                .or_insert_with(|| Candidate {
                    snapshot: current.clone(),
                    unchanged_since: now,
                });
            if candidate.snapshot != current {
                candidate.snapshot = current;
                candidate.unchanged_since = now;
            }
            if now.duration_since(candidate.unchanged_since)
                < Duration::from_secs(self.config.quiet_seconds)
            {
                continue;
            }
            match archive_checked(&path, &self.config.destination, Some(&candidate.snapshot)) {
                Ok(archive) => {
                    self.history.completed.insert(key);
                    // Stop on a history-write failure instead of silently losing duplicate tracking.
                    save_json(&self.history_path, &self.history)
                        .context("ZIP delivered, but completion history could not be saved")?;
                    self.candidates.remove(&path);
                    self.last_errors.remove(&path);
                    activity.push(Activity {
                        kind: "success".into(),
                        message: format!("Delivered {}", archive.display()),
                    });
                }
                Err(error) => self.report_error(&path, format!("{error:#}"), &mut activity),
            }
        }
        self.candidates.retain(|path, _| present.contains(path));
        self.last_errors.retain(|path, _| present.contains(path));
        Ok(activity)
    }

    fn report_error(&mut self, path: &Path, message: String, activity: &mut Vec<Activity>) {
        if self.last_errors.get(path) != Some(&message) {
            activity.push(Activity {
                kind: "error".into(),
                message: format!("{}: {message}", path.display()),
            });
            self.last_errors.insert(path.to_path_buf(), message);
        }
    }
}

pub fn save_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let parent = path.parent().context("Settings path has no parent")?;
    fs::create_dir_all(parent)?;
    let mut staged = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer_pretty(&mut staged, value)?;
    staged.write_all(b"\n")?;
    staged.as_file().sync_all()?;
    staged.persist(path).map_err(|error| error.error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::{TempDir, tempdir};

    fn fixture() -> (TempDir, Config, PathBuf) {
        let temp = tempdir().unwrap();
        let source = temp.path().join("orders");
        fs::create_dir_all(&source).unwrap();
        let config = Config {
            source,
            destination: temp.path().join("out"),
            quiet_seconds: 5,
        };
        let history = temp.path().join("data/history.json");
        (temp, config, history)
    }

    #[test]
    fn waits_for_changes_to_settle_and_remembers_delivery_after_zip_is_moved() {
        let (_temp, config, history) = fixture();
        let mut watcher = Watcher::new(config.clone(), history.clone()).unwrap();
        fs::create_dir_all(config.source.join("100")).unwrap();
        fs::write(config.source.join("100/invoice.txt"), "first").unwrap();
        let now = Instant::now();
        assert!(watcher.poll_at(now).unwrap().is_empty());
        fs::write(
            config.source.join("100/invoice.txt"),
            "still copying more content",
        )
        .unwrap();
        assert!(
            watcher
                .poll_at(now + Duration::from_secs(4))
                .unwrap()
                .is_empty()
        );
        assert!(
            watcher
                .poll_at(now + Duration::from_secs(8))
                .unwrap()
                .is_empty()
        );
        let activity = watcher.poll_at(now + Duration::from_secs(10)).unwrap();
        assert_eq!(activity.len(), 1);
        assert_eq!(activity[0].kind, "success");
        fs::remove_file(config.destination.join("100.zip")).unwrap();
        let mut restarted = Watcher::new(config.clone(), history).unwrap();
        assert!(restarted.poll_at(now).unwrap().is_empty());
        assert!(
            restarted
                .poll_at(now + Duration::from_secs(20))
                .unwrap()
                .is_empty()
        );
        assert!(!config.destination.join("100.zip").exists());
        assert_eq!(restarted.pending(), 0);
    }

    #[test]
    fn collision_does_not_block_other_orders_or_spam_errors() {
        let (_temp, config, history) = fixture();
        let mut watcher = Watcher::new(config.clone(), history).unwrap();
        fs::create_dir_all(config.source.join("100")).unwrap();
        fs::create_dir_all(config.source.join("200")).unwrap();
        fs::write(config.source.join("loose.txt"), "not an order").unwrap();
        fs::write(config.destination.join("100.zip"), "pre-existing").unwrap();
        let now = Instant::now();
        watcher.poll_at(now).unwrap();
        let activity = watcher.poll_at(now + Duration::from_secs(6)).unwrap();
        assert_eq!(
            activity
                .iter()
                .filter(|event| event.kind == "error")
                .count(),
            1
        );
        assert_eq!(
            activity
                .iter()
                .filter(|event| event.kind == "success")
                .count(),
            1
        );
        assert!(
            watcher
                .poll_at(now + Duration::from_secs(12))
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            fs::read_to_string(config.destination.join("100.zip")).unwrap(),
            "pre-existing"
        );
        assert!(!config.destination.join("loose.txt.zip").exists());
    }

    #[test]
    fn corrupt_history_and_overlapping_folders_are_rejected() {
        let (_temp, mut config, history) = fixture();
        fs::create_dir_all(history.parent().unwrap()).unwrap();
        fs::write(&history, "corrupt").unwrap();
        assert!(Watcher::new(config.clone(), history).is_err());
        config.destination = config.source.join("out");
        assert!(config.validate().is_err());
    }

    #[test]
    fn ignores_initial_folders_but_catches_arrivals_while_app_was_closed() {
        let (_temp, config, history) = fixture();
        fs::create_dir(config.source.join("old-order")).unwrap();
        let mut watcher = Watcher::new(config.clone(), history.clone()).unwrap();
        let now = Instant::now();
        assert!(watcher.poll_at(now).unwrap().is_empty());
        assert!(
            watcher
                .poll_at(now + Duration::from_secs(10))
                .unwrap()
                .is_empty()
        );
        assert_eq!(watcher.pending(), 0);
        drop(watcher);
        fs::create_dir(config.source.join("new-order")).unwrap();
        let mut watcher = Watcher::new(config.clone(), history).unwrap();
        assert!(watcher.poll_at(now).unwrap().is_empty());
        assert_eq!(
            watcher
                .poll_at(now + Duration::from_secs(10))
                .unwrap()
                .len(),
            1
        );
        assert!(!config.destination.join("old-order.zip").exists());
        assert!(config.destination.join("new-order.zip").exists());
    }
}
