//! Asking the terminal where its own window stands.
//!
//! Which of an emulator's windows this app is drawn in cannot be
//! settled from the outside. Every one of them answers to the same
//! application, so ownership does not tell them apart, and two opened
//! side by side are commonly the same size to the pixel, so neither
//! does size. The emulator itself knows, though, and xterm settled
//! long ago on a way to ask it: [`POSITION_QUERY`], answered with the
//! window's top-left corner.
//!
//! The answer arrives on this app's own input, ahead of anything the
//! reader has typed since. So it is read a byte at a time and stopped
//! at the reply's own terminator, and nothing past the reply is taken
//! out of the queue. A terminal with nothing to say costs one timed
//! wait rather than a read that never returns.
//!
//! # Invariants
//!
//! The terminal must be in raw mode and nothing else may be reading
//! its input. In cooked mode the reply is held back until a newline
//! that is never typed, and a second reader takes the bytes this one
//! is waiting for. Both end in the same place -- no answer, and the
//! caller falling back to what it did before -- so neither is an
//! error.

use std::io;
use std::io::Write;

use super::constants::POSITION_QUERY;

/// What the emulator said about where the window this app is drawn
/// in stands.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum TerminalWindowPosition {
    /// The terminal did not answer with a position.
    ///
    /// This covers every way the query can come to nothing: a terminal
    /// that does not know it, one that answers with something else, or
    /// a reply that arrived behind more input than the wait allows for.
    /// All of them mean the window has to be settled some other way.
    NotReported,
    /// The terminal answered, putting its window's top-left corner
    /// here in the window server's own point space.
    Reported {
        /// The window's top-left corner. Either coordinate may be
        /// negative: a display standing left of or above the main one
        /// puts every window on it at negative coordinates.
        origin: (f64, f64),
    },
}

impl TerminalWindowPosition {
    /// Read the terminal's position answer out of the bytes it sent
    /// back.
    fn in_reply(reply: &[u8]) -> Self {
        parse_position(reply).map_or(Self::NotReported, |origin| Self::Reported { origin })
    }
}

/// Ask where the emulator says the window this app is drawn in stands.
///
/// # Cost
///
/// One round trip over the pty. The write is flushed, so it also
/// carries whatever output was queued ahead of it -- which under a
/// running animation is most of what this waits for. Past the flush
/// the emulator has the query in hand and the reply is prompt.
pub(super) fn window_origin(out: &mut impl Write) -> TerminalWindowPosition {
    if ask(out).is_err() {
        return TerminalWindowPosition::NotReported;
    }
    TerminalWindowPosition::in_reply(&reply())
}

/// Put the query on the wire and see it out to the emulator.
fn ask(out: &mut impl Write) -> io::Result<()> {
    write!(out, "{POSITION_QUERY}")?;
    out.flush()
}

/// The position a reply carries, or [`None`] where `reply` is not one.
///
/// A reply is `CSI 3 ; x ; y t`, and both numbers may be negative: a
/// display standing left of or above the main one puts every window on
/// it at negative coordinates.
///
/// Anything ahead of the reply is dropped rather than refused. What
/// lands there is a keystroke that beat the reply out of the queue,
/// and losing one keystroke is a far better answer than losing the
/// window.
fn parse_position(reply: &[u8]) -> Option<(f64, f64)> {
    let start = reply.iter().rposition(|byte| *byte == 0x1b)?;
    let text = std::str::from_utf8(reply.get(start..)?).ok()?;
    let body = text.strip_prefix("\u{1b}[")?.strip_suffix('t')?;
    let mut fields = body.split(';');
    if fields.next()? != "3" {
        return None;
    }
    let x: i32 = fields.next()?.parse().ok()?;
    let y: i32 = fields.next()?.parse().ok()?;
    if fields.next().is_some() {
        return None;
    }
    Some((f64::from(x), f64::from(y)))
}

#[cfg(target_os = "macos")]
use platform::reply;

#[cfg(target_os = "macos")]
mod platform {
    use std::ffi::c_int;
    use std::time::Duration;
    use std::time::Instant;

    use crate::backdrop::constants::POSITION_REPLY_BYTES;
    use crate::backdrop::constants::POSITION_REPLY_END;
    use crate::backdrop::constants::POSITION_REPLY_WAIT;

    /// Whatever the terminal sent back, up to and including the
    /// reply's terminator.
    ///
    /// Read one byte at a time, which is what keeps this from taking
    /// anything that is not the reply: a read of many bytes would
    /// swallow every keystroke sitting behind it in the same queue.
    /// The byte cap is the backstop for a terminal that answers
    /// something with no terminator in it at all.
    ///
    /// Read through a descriptor of this query's own rather than
    /// through stdin. An app drawing a terminal UI has a thread parked
    /// in a read of its own on the keyboard, and the two are readers of
    /// one queue: `poll` reporting a byte is no promise it is still
    /// there a moment later, because the other reader is woken by the
    /// same byte and may take it first. On stdin, losing that race
    /// parks the caller in a read that nothing is coming for -- the
    /// terminal is in raw mode and the next byte is whenever the reader
    /// next presses a key, which for an animation meant to run
    /// untouched is never. The whole display stops, and stops looking
    /// like anything but an animation that never started.
    ///
    /// The race itself is not winnable from here, and does not need to
    /// be: a lost reply is a query that fails, which the caller already
    /// carries. What matters is that failing costs a wait rather than
    /// the process.
    pub(super) fn reply() -> Vec<u8> {
        let Some(tty) = Tty::open() else {
            return Vec::new();
        };
        let deadline = Instant::now() + POSITION_REPLY_WAIT;
        let mut reply = Vec::new();
        while reply.len() < POSITION_REPLY_BYTES {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() || !tty.readable(left) {
                break;
            }
            let Some(byte) = tty.next_byte() else {
                break;
            };
            reply.push(byte);
            if byte == POSITION_REPLY_END {
                break;
            }
        }
        reply
    }

    /// The controlling terminal, opened for this query alone and closed
    /// with the value.
    ///
    /// Its own descriptor, so `O_NONBLOCK` sits on it rather than on
    /// the one the app reads keystrokes through -- the flag belongs to
    /// the open file description, and setting it on a shared one would
    /// hand the keyboard reader an `EAGAIN` it treats as the input
    /// ending.
    struct Tty(c_int);

    impl Tty {
        /// Open the controlling terminal, or [`None`] where there is
        /// none to open -- a process with no terminal has no window to
        /// ask about either.
        #[allow(
            unsafe_code,
            reason = "opening a descriptor with the flags this needs                       has no safe binding: `File::open` cannot ask for                       `O_NONBLOCK`, and the standard library offers no                       way to set it afterwards"
        )]
        fn open() -> Option<Self> {
            // SAFETY: `open` reads the path it is handed as a C string,
            // and this one is a literal with its own terminator. The
            // flags are the two this reads with.
            let fd = unsafe { libc::open(c"/dev/tty".as_ptr(), libc::O_RDONLY | libc::O_NONBLOCK) };
            (fd >= 0).then_some(Self(fd))
        }

        /// Whether the terminal has something to say inside `within`.
        #[allow(
            unsafe_code,
            reason = "a timed wait on a descriptor has no safe binding                       in the standard library, which reads only without                       a deadline"
        )]
        fn readable(&self, within: Duration) -> bool {
            let mut watched = libc::pollfd {
                fd:      self.0,
                events:  libc::POLLIN,
                revents: 0,
            };
            let millis = i32::try_from(within.as_millis()).unwrap_or(i32::MAX);
            // SAFETY: `poll` reads and writes the array it is handed for
            // exactly the count that comes with it, and that count is
            // one for the one `pollfd` this pointer is to. It writes
            // only `revents`, which the wait below is the only reader
            // of.
            let ready = unsafe { libc::poll(&raw mut watched, 1, millis) };
            ready > 0 && watched.revents & libc::POLLIN != 0
        }

        /// One byte of the terminal's answer, or [`None`] where there
        /// is none to be had.
        ///
        /// Cannot wait: the descriptor carries `O_NONBLOCK`, so a byte
        /// another reader took between the poll and here comes back as
        /// an error rather than as a read that never returns.
        #[allow(
            unsafe_code,
            reason = "reading the raw descriptor is what keeps the read                       to the single byte `readable` reported, where the                       standard library's own buffering would take the                       reader's keystrokes along with it"
        )]
        fn next_byte(&self) -> Option<u8> {
            let mut read = [0_u8; 1];
            // SAFETY: `read` writes at most the byte count it is
            // handed, and that count is one for the one-byte array this
            // pointer is to.
            let taken = unsafe { libc::read(self.0, read.as_mut_ptr().cast(), 1) };
            (taken == 1).then(|| read[0])
        }
    }

    impl Drop for Tty {
        #[allow(
            unsafe_code,
            reason = "the descriptor was opened by hand above and has                       no owning handle to close it"
        )]
        fn drop(&mut self) {
            // SAFETY: `self.0` came from `open` above and is closed
            // once, here, because nothing else holds a copy of it.
            unsafe {
                libc::close(self.0);
            }
        }
    }
}

/// Nothing answers the query where there are no windows to ask about.
#[cfg(not(target_os = "macos"))]
const fn reply() -> Vec<u8> { Vec::new() }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_position_out_of_a_reply() {
        assert_eq!(
            TerminalWindowPosition::in_reply(b"\x1b[3;1720;0t"),
            TerminalWindowPosition::Reported {
                origin: (1720.0, 0.0),
            },
            "the reply iTerm2 sends should read as the window's corner"
        );
    }

    #[test]
    fn reads_a_window_on_a_display_left_of_the_main_one() {
        assert_eq!(
            TerminalWindowPosition::in_reply(b"\x1b[3;-1720;-200t"),
            TerminalWindowPosition::Reported {
                origin: (-1720.0, -200.0),
            },
            "a display left of and above the main one puts its windows \
             at negative coordinates"
        );
    }

    #[test]
    fn drops_a_keystroke_that_beat_the_reply_out_of_the_queue() {
        assert_eq!(
            TerminalWindowPosition::in_reply(b"a\x1b[3;12;34t"),
            TerminalWindowPosition::Reported {
                origin: (12.0, 34.0),
            },
            "input typed before the reply arrived should not cost the \
             window"
        );
        assert_eq!(
            TerminalWindowPosition::in_reply(b"\x1b[A\x1b[3;12;34t"),
            TerminalWindowPosition::Reported {
                origin: (12.0, 34.0),
            },
            "an arrow key is itself an escape sequence, and the reply \
             is the last one in the queue"
        );
    }

    #[test]
    fn refuses_a_reply_that_is_not_a_position() {
        assert_eq!(
            TerminalWindowPosition::in_reply(b"\x1b[8;87;244t"),
            TerminalWindowPosition::NotReported,
            "the size report shares the reply's grammar and answers a \
             different question"
        );
        assert_eq!(
            TerminalWindowPosition::in_reply(b"\x1b[3;1720;0;9t"),
            TerminalWindowPosition::NotReported,
            "a position carries two numbers and no more"
        );
        assert_eq!(
            TerminalWindowPosition::in_reply(b"\x1b[3;1720t"),
            TerminalWindowPosition::NotReported,
            "a position carries two numbers and no fewer"
        );
        assert_eq!(
            TerminalWindowPosition::in_reply(b"\x1b[3;wide;0t"),
            TerminalWindowPosition::NotReported,
            "a coordinate that is not a number is not a position"
        );
    }

    #[test]
    fn refuses_a_terminal_that_said_nothing() {
        assert_eq!(
            TerminalWindowPosition::in_reply(b""),
            TerminalWindowPosition::NotReported,
            "a terminal that does not know the query answers nothing \
             at all"
        );
        assert_eq!(
            TerminalWindowPosition::in_reply(b"\x1b[3;1720;0"),
            TerminalWindowPosition::NotReported,
            "a reply cut off before its terminator is one the wait ran \
             out on"
        );
    }
}
