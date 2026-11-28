//! `WordHighlightUpdates` reads configuration and sends `WordHighlightUpdate` values to the
//! terminal loop.

use std::fs;
use std::io;
use std::io::ErrorKind;
#[cfg(target_os = "macos")]
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::mpsc::Receiver;
use std::sync::mpsc::Sender;
use std::sync::mpsc::TryRecvError;
use std::thread;
use std::thread::JoinHandle;
#[cfg(test)]
use std::time::Duration;

#[cfg(any(not(target_os = "macos"), test))]
use notify::Event;
#[cfg(not(target_os = "macos"))]
use notify::RecursiveMode;
#[cfg(not(target_os = "macos"))]
use notify::Watcher;

#[cfg(target_os = "macos")]
use super::macos;
use crate::config::Config;
use crate::config::WordHighlightEmphasis;

/// `WordHighlightUpdate` carries a parsed emphasis or a configuration error.
#[derive(Debug)]
pub(crate) enum WordHighlightUpdate {
    Valid(WordHighlightEmphasis),
    Invalid(String),
}

#[cfg(not(target_os = "macos"))]
enum ConfigFileEvent {
    Filesystem(notify::Result<Event>),
    Stop,
}

/// `WordHighlightUpdates` owns the reader thread, its delivery channel, and its shutdown request.
#[derive(Default)]
pub(crate) enum WordHighlightUpdates {
    #[default]
    Disabled,
    Watching {
        updates: Receiver<WordHighlightUpdate>,
        reader:  Option<ConfigReaderThread>,
    },
}

#[cfg(target_os = "macos")]
type ConfigReaderStop = UnixStream;
#[cfg(not(target_os = "macos"))]
type ConfigReaderStop = Sender<ConfigFileEvent>;

/// `ConfigReaderThread` owns the reader and the endpoint that stops it.
pub(crate) struct ConfigReaderThread {
    stop:   ConfigReaderStop,
    handle: JoinHandle<()>,
}

#[cfg(test)]
const TEST_CONFIG_UPDATE_TIMEOUT: Duration = Duration::from_millis(500);

impl WordHighlightUpdates {
    /// `WordHighlightUpdates::spawn` watches the directory across atomic file replacements.
    pub(crate) fn spawn(path: PathBuf) -> Self {
        let (sender, updates) = mpsc::channel();
        let reader = ConfigReaderThread::spawn(path, &sender).map_or_else(
            |error| {
                let _ = sender.send(WordHighlightUpdate::Invalid(error.to_string()));
                None
            },
            Some,
        );
        Self::Watching { updates, reader }
    }

    /// `WordHighlightUpdates::try_recv` reads completed updates without filesystem access or a
    /// blocking receive.
    pub(crate) fn try_recv(&self) -> Result<WordHighlightUpdate, TryRecvError> {
        match self {
            Self::Disabled => Err(TryRecvError::Disconnected),
            Self::Watching { updates, .. } => updates.try_recv(),
        }
    }

    #[cfg(test)]
    pub(crate) fn receive_for_test(
        &self,
        step: &str,
    ) -> Result<WordHighlightUpdate, Box<dyn std::error::Error>> {
        match self {
            Self::Disabled => Err(format!("{step}: the configuration reader is disabled").into()),
            Self::Watching { updates, .. } => updates
                .recv_timeout(TEST_CONFIG_UPDATE_TIMEOUT)
                .map_err(|error| format!("{step}: {error}").into()),
        }
    }
}

impl Drop for WordHighlightUpdates {
    fn drop(&mut self) {
        if let Self::Watching { reader, .. } = self
            && let Some(reader) = reader.take()
        {
            reader.stop();
        }
    }
}

impl ConfigReaderThread {
    fn spawn(path: PathBuf, sender: &Sender<WordHighlightUpdate>) -> io::Result<Self> {
        #[cfg(target_os = "macos")]
        let (stop, changes) = UnixStream::pair()?;
        #[cfg(not(target_os = "macos"))]
        let (stop, events) = mpsc::channel();
        #[cfg(not(target_os = "macos"))]
        let changes = (stop.clone(), events);
        let sender = sender.clone();
        let handle = thread::Builder::new().spawn(move || {
            #[cfg(target_os = "macos")]
            let result = {
                let mut reading = ConfigFileReading::default();
                macos::watch_changes(&path, &changes, || reading.publish(&path, &sender))
                    .map_err(|error| format!("{}: {error}", path.display()))
            };
            #[cfg(not(target_os = "macos"))]
            let result = read_changes(&path, changes, &sender);
            if let Err(error) = result {
                let _ = sender.send(WordHighlightUpdate::Invalid(error));
            }
        })?;
        Ok(Self { stop, handle })
    }

    fn stop(self) {
        #[cfg(not(target_os = "macos"))]
        let _ = self.stop.send(ConfigFileEvent::Stop);
        drop(self.stop);
        let _ = self.handle.join();
    }
}

/// `ConfigFileReading` identifies file changes independently of timestamp precision.
#[derive(Default)]
enum ConfigFileReading {
    #[default]
    Unread,
    Text(String),
    Failed(String),
}

impl ConfigFileReading {
    fn publish(&mut self, path: &Path, sender: &Sender<WordHighlightUpdate>) {
        let reading = fs::read_to_string(path);
        let update = match reading {
            Ok(text) => {
                if matches!(self, Self::Text(previous) if previous == &text) {
                    return;
                }
                let result = toml::from_str::<Config>(&text).map_or_else(
                    |error| WordHighlightUpdate::Invalid(error.to_string()),
                    |config| WordHighlightUpdate::Valid(config.word_highlight.emphasis),
                );
                *self = Self::Text(text);
                result
            },
            Err(error) => {
                if matches!(self, Self::Unread) && error.kind() == ErrorKind::NotFound {
                    return;
                }
                let message = format!("{}: {error}", path.display());
                if matches!(self, Self::Failed(previous) if previous == &message) {
                    return;
                }
                *self = Self::Failed(message.clone());
                WordHighlightUpdate::Invalid(message)
            },
        };
        let _ = sender.send(update);
    }

    #[cfg(any(not(target_os = "macos"), test))]
    fn handle_event(
        &mut self,
        path: &Path,
        event: notify::Result<Event>,
        sender: &Sender<WordHighlightUpdate>,
        refresh_watch: impl FnOnce() -> Result<(), String>,
    ) -> Result<(), String> {
        match event {
            Ok(event) if event.kind.is_access() => {},
            Ok(_) => {
                refresh_watch()?;
                self.publish(path, sender);
            },
            Err(error) => {
                let _ = sender.send(WordHighlightUpdate::Invalid(error.to_string()));
            },
        }
        Ok(())
    }
}

/// Publish file contents with the same reader that serves the watcher.
#[cfg(test)]
#[derive(Default)]
pub(crate) struct ConfigFileReadFixture {
    reading: ConfigFileReading,
}

#[cfg(test)]
impl ConfigFileReadFixture {
    pub(crate) fn read(
        &mut self,
        path: &Path,
        step: &str,
    ) -> Result<WordHighlightUpdate, Box<dyn std::error::Error>> {
        self.update(step, |reading, sender| {
            reading.publish(path, sender);
            Ok(())
        })
    }

    pub(crate) fn read_after_event(
        &mut self,
        path: &Path,
        event: Event,
        step: &str,
    ) -> Result<WordHighlightUpdate, Box<dyn std::error::Error>> {
        self.update(step, |reading, sender| {
            reading.handle_event(path, Ok(event), sender, || Ok(()))
        })
    }

    fn update(
        &mut self,
        step: &str,
        publish: impl FnOnce(&mut ConfigFileReading, &Sender<WordHighlightUpdate>) -> Result<(), String>,
    ) -> Result<WordHighlightUpdate, Box<dyn std::error::Error>> {
        let (sender, updates) = mpsc::channel();
        publish(&mut self.reading, &sender).map_err(|error| format!("{step}: {error}"))?;
        updates
            .try_recv()
            .map_err(|error| format!("{step}: {error}").into())
    }
}

#[cfg(not(target_os = "macos"))]
fn watch_directory(path: &Path) -> Option<&Path> {
    path.ancestors().skip(1).find(|ancestor| ancestor.is_dir())
}

#[cfg(not(target_os = "macos"))]
fn read_changes(
    path: &Path,
    changes: (Sender<ConfigFileEvent>, Receiver<ConfigFileEvent>),
    sender: &Sender<WordHighlightUpdate>,
) -> Result<(), String> {
    let (notifications, events) = changes;
    let mut watcher = notify::recommended_watcher(move |event| {
        let _ = notifications.send(ConfigFileEvent::Filesystem(event));
    })
    .map_err(|error| error.to_string())?;
    let directory = || {
        watch_directory(path).map(Path::to_path_buf).ok_or_else(|| {
            format!(
                "{} has no available configuration directory",
                path.display()
            )
        })
    };
    let mut source_directory = directory()?;
    watcher
        .watch(&source_directory, RecursiveMode::NonRecursive)
        .map_err(|error| error.to_string())?;
    let mut reading = ConfigFileReading::default();
    reading.publish(path, sender);
    while let Ok(ConfigFileEvent::Filesystem(event)) = events.recv() {
        reading.handle_event(path, event, sender, || {
            let current = directory()?;
            if current != source_directory {
                watcher
                    .watch(&current, RecursiveMode::NonRecursive)
                    .map_err(|error| error.to_string())?;
                let _ = watcher.unwatch(&source_directory);
                source_directory = current;
            }
            Ok(())
        })?;
    }
    Ok(())
}
