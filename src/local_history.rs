//! Shared native Reedline history. No shell-history import or implicit file path.
use crate::context::{MAX_CONTEXT_BYTES, unsafe_char};
use reedline::{
    FileBackedHistory, History, HistoryItem, HistoryItemId, HistorySessionId, SearchDirection,
    SearchQuery,
};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

pub const HISTORY_CAPACITY: usize = 10_000;

#[derive(Clone)]
pub struct LocalHistory(
    Arc<Mutex<FileBackedHistory>>,
    Option<PathBuf>,
    Arc<std::sync::atomic::AtomicBool>,
);
impl LocalHistory {
    pub fn new(path: Option<PathBuf>) -> std::io::Result<Self> {
        let inner = match path.as_ref() {
            Some(path) => {
                ensure_private_file(path)?;
                validate_file(path)?;
                FileBackedHistory::with_file(HISTORY_CAPACITY, path.clone())?
            }
            None => FileBackedHistory::new(HISTORY_CAPACITY)?,
        };
        let contains_unsafe = contains_unsafe(&inner)?;
        Ok(Self(
            Arc::new(Mutex::new(inner)),
            path,
            Arc::new(std::sync::atomic::AtomicBool::new(contains_unsafe)),
        ))
    }
    /// Preview/headless paths must not create, truncate, sync, or rewrite a file.
    pub fn read_only(path: Option<PathBuf>) -> std::io::Result<Self> {
        use std::io::BufRead;
        let mut history = Self::new(None)?;
        if let Some(path) = path {
            validate_file(&path)?;
            let file = std::fs::File::open(path)?;
            for line in std::io::BufReader::new(file).lines() {
                // Native FileBackedHistory encodes embedded newlines as <\n>.
                // This single-line app rejects decoded multiline entries.
                let line = line?.replace("<\\n>", "\n");
                history.save(HistoryItem::from_command_line(line))?;
            }
        }
        Ok(history)
    }

    /// Bounded snapshot, oldest first; only safe entries can reach UI or model.
    pub fn recent(&self, limit: i64) -> Vec<String> {
        let mut query = SearchQuery::everything(SearchDirection::Backward, None);
        query.limit = Some(limit);
        let mut lines: Vec<_> = self
            .search(query)
            .unwrap_or_default()
            .into_iter()
            .map(|h| h.command_line)
            .collect();
        lines.reverse();
        lines
    }
}
impl Drop for LocalHistory {
    fn drop(&mut self) {
        if Arc::strong_count(&self.0) == 1 {
            let _ = self.sync();
        }
    }
}
fn contains_unsafe(history: &FileBackedHistory) -> std::io::Result<bool> {
    Ok(history
        .search(SearchQuery::everything(SearchDirection::Backward, None))?
        .iter()
        .any(|h| !safe(&h.command_line)))
}
fn ensure_private_file(path: &std::path::Path) -> std::io::Result<()> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    match options.open(path) {
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(error),
    }
}
fn validate_file(path: &std::path::Path) -> std::io::Result<()> {
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.file_type().is_file() || metadata.len() > 128 * 1024 * 1024 {
        return Err(std::io::Error::other(
            "history must be a regular file of at most 128 MiB",
        ));
    }
    Ok(())
}
fn safe(line: &str) -> bool {
    !line.is_empty()
        && !line.starts_with([':', ' '])
        && line.len() <= MAX_CONTEXT_BYTES
        && !line.chars().any(unsafe_char)
}
fn invalid() -> reedline::ReedlineError {
    reedline::ReedlineError(reedline::ReedlineErrorVariants::OtherHistoryError(
        "unsafe history entry rejected",
    ))
}
impl History for LocalHistory {
    fn save(&mut self, item: HistoryItem) -> reedline::Result<HistoryItem> {
        // App controls and leading-space entries never enter local history.
        if !safe(&item.command_line) || item.command_line.starts_with([':', ' ']) {
            return Ok(item);
        }
        self.0.lock().expect("history lock").save(item)
    }
    fn load(&self, id: HistoryItemId) -> reedline::Result<HistoryItem> {
        let item = self.0.lock().expect("history lock").load(id)?;
        if safe(&item.command_line) {
            Ok(item)
        } else {
            Err(invalid())
        }
    }
    fn count(&self, query: SearchQuery) -> reedline::Result<i64> {
        Ok(self.search(query)?.len() as i64)
    }
    fn search(&self, mut query: SearchQuery) -> reedline::Result<Vec<HistoryItem>> {
        // Preserve native limit=1 for the normal, validated local history path.
        // Only externally contaminated files need a scan past invalid entries.
        let limit = query.limit.unwrap_or(i64::MAX).max(0) as usize;
        if self.2.load(std::sync::atomic::Ordering::Relaxed) {
            query.limit = None;
        }
        Ok(self
            .0
            .lock()
            .expect("history lock")
            .search(query)?
            .into_iter()
            .filter(|h| safe(&h.command_line))
            .take(limit)
            .collect())
    }
    fn update(
        &mut self,
        id: HistoryItemId,
        updater: &dyn Fn(HistoryItem) -> HistoryItem,
    ) -> reedline::Result<()> {
        let item = updater(self.load(id)?);
        if !safe(&item.command_line) {
            return Err(invalid());
        }
        self.0
            .lock()
            .expect("history lock")
            .update(id, &|_| item.clone())
    }
    fn clear(&mut self) -> reedline::Result<()> {
        self.0.lock().expect("history lock").clear()?;
        if let Some(path) = &self.1 {
            ensure_private_file(path)?;
        }
        self.2.store(false, std::sync::atomic::Ordering::Relaxed);
        Ok(())
    }
    fn delete(&mut self, id: HistoryItemId) -> reedline::Result<()> {
        self.0.lock().expect("history lock").delete(id)
    }
    fn sync(&mut self) -> std::io::Result<()> {
        if self.1.is_none() {
            return Ok(());
        }
        if let Some(path) = &self.1 {
            ensure_private_file(path)?;
            validate_file(path)?;
        }
        let mut inner = self.0.lock().expect("history lock");
        inner.sync()?;
        self.2.store(
            contains_unsafe(&inner)?,
            std::sync::atomic::Ordering::Relaxed,
        );
        Ok(())
    }
    fn session(&self) -> Option<HistorySessionId> {
        self.0.lock().expect("history lock").session()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_history_is_shared_bounded_and_skips_controls() {
        let mut history = LocalHistory::new(None).unwrap();
        let clone = history.clone();
        for line in [
            "echo first",
            "echo second",
            ":history-preview",
            " secret",
            "bad\x1b",
        ] {
            history.save(HistoryItem::from_command_line(line)).unwrap();
        }
        assert_eq!(clone.recent(1), ["echo second"]);
        assert_eq!(history.count_all().unwrap(), 2);
        history.clear().unwrap();
        assert!(clone.recent(500).is_empty());
    }
}

#[cfg(test)]
mod persistence_tests {
    use super::*;
    #[test]
    fn explicit_persistence_survives_restart_and_clear_and_rejects_unsafe_entries() {
        let dir = std::env::temp_dir().join(format!(
            "reedline-native-persistence-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("history");
        {
            let mut history = LocalHistory::new(Some(path.clone())).unwrap();
            history
                .save(HistoryItem::from_command_line("echo synthetic-persisted"))
                .unwrap();
            history.sync().unwrap();
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        {
            let mut history = LocalHistory::new(Some(path.clone())).unwrap();
            assert_eq!(history.recent(10), ["echo synthetic-persisted"]);
            history.clear().unwrap();
            history.sync().unwrap();
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        assert!(
            LocalHistory::read_only(Some(path.clone()))
                .unwrap()
                .recent(10)
                .is_empty()
        );
        std::fs::write(
            &path,
            "echo safe\nbad\x1b[2J\n:history-clear\n secret\nline<\\n>break\n",
        )
        .unwrap();
        assert_eq!(
            LocalHistory::read_only(Some(path.clone()))
                .unwrap()
                .recent(10),
            ["echo safe"]
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
