//! `WordHighlightUpdates` reads configuration and sends `WordHighlightUpdate` values to the
//! terminal loop.

use std::fs;
use std::io::ErrorKind;
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

use notify::Event;
use notify::RecursiveMode;
use notify::Watcher;

use crate::config::Config;
use crate::config::WordHighlightEmphasis;

/// `WordHighlightUpdate` carries a parsed emphasis or a configuration error.
#[derive(Debug)]
pub(crate) enum WordHighlightUpdate {
    Valid(WordHighlightEmphasis),
    Invalid(String),
}

pub(crate) enum ConfigFileEvent {
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
        stop:    Sender<ConfigFileEvent>,
        reader:  ConfigReaderThread,
    },
}

/// `ConfigReaderThread` records whether the reader still requires a join.
pub(crate) enum ConfigReaderThread {
    Running(JoinHandle<()>),
    Joined,
}

#[cfg(test)]
const TEST_CONFIG_UPDATE_TIMEOUT: Duration = Duration::from_millis(500);

impl WordHighlightUpdates {
    /// `WordHighlightUpdates::spawn` watches the directory across atomic file replacements.
    pub(crate) fn spawn(path: PathBuf) -> Self {
        let (sender, updates) = mpsc::channel();
        let (stop, events) = mpsc::channel();
        let notifications = stop.clone();
        let reader = thread::spawn(move || {
            let result = read_changes(&path, notifications, &events, &sender);
            if let Err(error) = result {
                let _ = sender.send(WordHighlightUpdate::Invalid(error));
            }
        });
        Self::Watching {
            updates,
            stop,
            reader: ConfigReaderThread::Running(reader),
        }
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
    ) -> Result<WordHighlightUpdate, Box<dyn std::error::Error>> {
        match self {
            Self::Disabled => Err("the configuration reader is disabled".into()),
            Self::Watching { updates, .. } => Ok(updates.recv_timeout(TEST_CONFIG_UPDATE_TIMEOUT)?),
        }
    }
}

impl Drop for WordHighlightUpdates {
    fn drop(&mut self) {
        if let Self::Watching { stop, reader, .. } = self {
            let _ = stop.send(ConfigFileEvent::Stop);
            if let ConfigReaderThread::Running(handle) =
                std::mem::replace(reader, ConfigReaderThread::Joined)
            {
                let _ = handle.join();
            }
        }
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
}

fn read_changes(
    path: &Path,
    notifications: Sender<ConfigFileEvent>,
    events: &Receiver<ConfigFileEvent>,
    sender: &Sender<WordHighlightUpdate>,
) -> Result<(), String> {
    let mut watcher = notify::recommended_watcher(move |event| {
        let _ = notifications.send(ConfigFileEvent::Filesystem(event));
    })
    .map_err(|error| error.to_string())?;
    let directory = || {
        path.ancestors()
            .skip(1)
            .find(|ancestor| ancestor.is_dir())
            .map(Path::to_path_buf)
            .ok_or_else(|| {
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
        match event {
            Ok(event) if event.kind.is_access() => {},
            Ok(_) => {
                let current = directory()?;
                if current != source_directory {
                    watcher
                        .watch(&current, RecursiveMode::NonRecursive)
                        .map_err(|error| error.to_string())?;
                    let _ = watcher.unwatch(&source_directory);
                    source_directory = current;
                }
                reading.publish(path, sender);
            },
            Err(error) => {
                let _ = sender.send(WordHighlightUpdate::Invalid(error.to_string()));
            },
        }
    }
    Ok(())
}
