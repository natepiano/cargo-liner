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

/// Where the emulator says the window this app is drawn in stands, in
/// the window server's own point space, or [`None`] where the terminal
/// did not answer with a position.
///
/// [`None`] covers every way this can come to nothing: a terminal that
/// does not know the query, one that answers with something else, a
/// reply that arrived behind more input than the wait allows for. All
/// of them mean the same thing to the caller, which is that the window
/// has to be settled some other way.
///
/// # Cost
///
/// One round trip over the pty. The write is flushed, so it also
/// carries whatever output was queued ahead of it -- which under a
/// running animation is most of what this waits for. Past the flush
/// the emulator has the query in hand and the reply is prompt.
pub(super) fn window_origin(out: &mut impl Write) -> Option<(f64, f64)> {
    ask(out).ok()?;
    parse_position(&reply())
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
    use std::time::Duration;
    use std::time::Instant;

    use super::super::constants::POSITION_REPLY_BYTES;
    use super::super::constants::POSITION_REPLY_END;
    use super::super::constants::POSITION_REPLY_WAIT;

    /// Whatever the terminal sent back, up to and including the
    /// reply's terminator.
    ///
    /// Read one byte at a time, which is what keeps this from taking
    /// anything that is not the reply: a read of many bytes would
    /// swallow every keystroke sitting behind it in the same queue.
    /// The byte cap is the backstop for a terminal that answers
    /// something with no terminator in it at all.
    pub(super) fn reply() -> Vec<u8> {
        let deadline = Instant::now() + POSITION_REPLY_WAIT;
        let mut reply = Vec::new();
        while reply.len() < POSITION_REPLY_BYTES {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() || !readable(left) {
                break;
            }
            let Some(byte) = next_byte() else {
                break;
            };
            reply.push(byte);
            if byte == POSITION_REPLY_END {
                break;
            }
        }
        reply
    }

    /// Whether the terminal has something to say inside `within`.
    #[allow(
        unsafe_code,
        reason = "a timed wait on this app's own input has no safe \
                  binding: the standard library reads stdin only \
                  without a deadline, and a read that never returns is \
                  what a terminal ignoring the query would leave"
    )]
    fn readable(within: Duration) -> bool {
        let mut watched = libc::pollfd {
            fd:      libc::STDIN_FILENO,
            events:  libc::POLLIN,
            revents: 0,
        };
        let millis = i32::try_from(within.as_millis()).unwrap_or(i32::MAX);
        // SAFETY: `poll` reads and writes the array it is handed for
        // exactly the count that comes with it, and that count is one
        // for the one `pollfd` this pointer is to. It writes only
        // `revents`, which the wait below is the only reader of.
        let ready = unsafe { libc::poll(&raw mut watched, 1, millis) };
        ready > 0 && watched.revents & libc::POLLIN != 0
    }

    /// One byte of the terminal's answer, or [`None`] at the end of
    /// the input or on an error reading it.
    ///
    /// Only ever called behind [`readable`], so it does not wait.
    #[allow(
        unsafe_code,
        reason = "reading the raw descriptor is what keeps the read to \
                  the single byte `readable` reported, where the \
                  standard library's own buffering would take the \
                  reader's keystrokes along with it"
    )]
    fn next_byte() -> Option<u8> {
        let mut read = [0_u8; 1];
        // SAFETY: `read` writes at most the byte count it is handed,
        // and that count is one for the one-byte array this pointer is
        // to.
        let taken = unsafe { libc::read(libc::STDIN_FILENO, read.as_mut_ptr().cast(), 1) };
        (taken == 1).then(|| read[0])
    }
}

/// Nothing answers the query where there are no windows to ask about.
#[cfg(not(target_os = "macos"))]
fn reply() -> Vec<u8> { Vec::new() }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_position_out_of_a_reply() {
        assert_eq!(
            parse_position(b"\x1b[3;1720;0t"),
            Some((1720.0, 0.0)),
            "the reply iTerm2 sends should read as the window's corner"
        );
    }

    #[test]
    fn reads_a_window_on_a_display_left_of_the_main_one() {
        assert_eq!(
            parse_position(b"\x1b[3;-1720;-200t"),
            Some((-1720.0, -200.0)),
            "a display left of and above the main one puts its windows \
             at negative coordinates"
        );
    }

    #[test]
    fn drops_a_keystroke_that_beat_the_reply_out_of_the_queue() {
        assert_eq!(
            parse_position(b"a\x1b[3;12;34t"),
            Some((12.0, 34.0)),
            "input typed before the reply arrived should not cost the \
             window"
        );
        assert_eq!(
            parse_position(b"\x1b[A\x1b[3;12;34t"),
            Some((12.0, 34.0)),
            "an arrow key is itself an escape sequence, and the reply \
             is the last one in the queue"
        );
    }

    #[test]
    fn refuses_a_reply_that_is_not_a_position() {
        assert_eq!(
            parse_position(b"\x1b[8;87;244t"),
            None,
            "the size report shares the reply's grammar and answers a \
             different question"
        );
        assert_eq!(
            parse_position(b"\x1b[3;1720;0;9t"),
            None,
            "a position carries two numbers and no more"
        );
        assert_eq!(
            parse_position(b"\x1b[3;1720t"),
            None,
            "a position carries two numbers and no fewer"
        );
        assert_eq!(
            parse_position(b"\x1b[3;wide;0t"),
            None,
            "a coordinate that is not a number is not a position"
        );
    }

    #[test]
    fn refuses_a_terminal_that_said_nothing() {
        assert_eq!(
            parse_position(b""),
            None,
            "a terminal that does not know the query answers nothing \
             at all"
        );
        assert_eq!(
            parse_position(b"\x1b[3;1720;0"),
            None,
            "a reply cut off before its terminator is one the wait ran \
             out on"
        );
    }
}
