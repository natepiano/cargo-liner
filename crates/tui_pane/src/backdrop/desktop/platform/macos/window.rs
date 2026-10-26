use std::collections::HashSet;
use std::env;
use std::ffi::c_void;

use objc2_core_foundation::CFArray;
use objc2_core_foundation::CFDictionary;
use objc2_core_foundation::CFNumber;
use objc2_core_foundation::CFString;
use objc2_core_foundation::CFType;
use objc2_core_foundation::CGRect as CoreGraphicsRect;
use objc2_core_graphics::CGRectMakeWithDictionaryRepresentation;
use objc2_core_graphics::CGWindowListCopyWindowInfo;
use objc2_core_graphics::CGWindowListOption;
use objc2_core_graphics::kCGNullWindowID;
use objc2_core_graphics::kCGWindowBounds;
use objc2_core_graphics::kCGWindowLayer;
use objc2_core_graphics::kCGWindowName;
use objc2_core_graphics::kCGWindowNumber;
use objc2_core_graphics::kCGWindowOwnerName;
use objc2_core_graphics::kCGWindowOwnerPID;
use sysinfo::Pid;
use sysinfo::ProcessRefreshKind;
use sysinfo::ProcessesToUpdate;
use sysinfo::System;

use super::display::Display;
use super::display::backing_scale;
use super::display::display_under;
use crate::backdrop::constants::EMULATOR_NAME_FLOOR;
use crate::backdrop::constants::POSITION_TOLERANCE;
use crate::backdrop::constants::TERM_PROGRAM_ENV;
use crate::backdrop::desktop::Frame;
use crate::backdrop::desktop::TerminalWindowSearchOutcome;
use crate::backdrop::desktop::TitledWindow;
use crate::backdrop::desktop::WindowTitle;
use crate::backdrop::desktop::candidate;
use crate::backdrop::desktop::candidate::TerminalWindowCandidate;
use crate::backdrop::desktop::candidate::TerminalWindowCandidates;
use crate::backdrop::desktop::candidate::TerminalWindowOwner;
use crate::process;

/// Which of the entries describing a window is wanted.
///
/// The names are `CFString` constants the framework builds as it
/// loads, and reading an `extern` static is unchecked -- so they
/// are read in the one place below rather than at every use.
#[derive(Clone, Copy)]
enum Key {
    /// The window server's own number for the window.
    Number,
    /// Where the window stands and how big it is.
    Bounds,
    /// Which layer it is drawn on. Ordinary windows are on nought;
    /// the menu bar, the dock and the desktop picture are not.
    Layer,
    /// The pid of the application that owns it.
    Owner,
    /// The name that application answers to.
    OwnerName,
    /// What the window is titled.
    Title,
}

impl Key {
    /// The CoreGraphics constant naming this entry.
    #[allow(
        unsafe_code,
        reason = "an `extern` static has no safe binding; the SAFETY \
                  comment below covers every one of them at once"
    )]
    fn name(self) -> &'static CFString {
        // SAFETY: `Key::name` reads an `extern` static, which is unchecked because
        // nothing on this side knows the symbol was ever
        // initialised. These are `CFString` constants CoreGraphics
        // builds as the framework loads -- which is before any code
        // that could reach this line -- and keeps for the life of
        // the process, so whichever reference is read out is live
        // and cannot dangle while the caller holds it.
        unsafe {
            match self {
                Self::Number => kCGWindowNumber,
                Self::Bounds => kCGWindowBounds,
                Self::Layer => kCGWindowLayer,
                Self::Owner => kCGWindowOwnerPID,
                Self::OwnerName => kCGWindowOwnerName,
                Self::Title => kCGWindowName,
            }
        }
    }
}

/// Whether the window server numbered a window in the dictionary
/// describing it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DescribedWindowNumber {
    /// The description carried no window number a caller could use:
    /// the key was absent, held something other than a number, or held
    /// one no `u32` can represent. A window that cannot be named is of
    /// no use to either caller, so both drop it.
    Unnumbered,
    /// The window server numbers the described window this way.
    Numbered {
        /// The window server's own number for the window.
        window_id: u32,
    },
}

/// Application-name facts used to match a window to `TERM_PROGRAM`.
pub(super) trait TerminalProgramWindowCandidate {
    /// Every folded name the owning application answers to.
    fn names(&self) -> Vec<String>;
}

/// One window of a `CGWindowList` answer, read out of the
/// dictionary describing it.
pub(super) struct Listed {
    /// The window server's own number for it.
    pub(super) number: u32,
    /// Which application owns it. [`TerminalWindowOwner::Unnamed`]
    /// means the dictionary named no owner to match to a process.
    pub(super) owner:  TerminalWindowOwner,
    /// The name that application answers to, folded.
    name:              Option<String>,
    /// What it is titled. [`window_at`] is the path that does not need
    /// the window server to report a title.
    title:             WindowTitle,
    /// Where it stands, in the space `CGDisplayBounds` measures.
    pub(super) bounds: CoreGraphicsRect,
    /// Which layer it is drawn on.
    layer:             i32,
}

impl Listed {
    /// Every window on screen, in the front-to-back order the
    /// window server lists them in.
    ///
    /// This is the cheap way of asking, and the reason the drawing
    /// thread can ask at all: `SCShareableContent::get` describes
    /// the same windows and takes about seventy milliseconds over
    /// it, which is a frame lost every time it is called and was
    /// several frames lost every pass. A run that cannot draw looks
    /// exactly like one whose screen never came on. See
    /// [`window_frame`] for the same comparison.
    pub(super) fn on_screen() -> Vec<Self> {
        let Some(list) =
            CGWindowListCopyWindowInfo(CGWindowListOption::OptionOnScreenOnly, kCGNullWindowID)
        else {
            return Vec::new();
        };
        (0..list.count())
            .filter_map(|index| Self::read(entry(&list, index)?))
            .collect()
    }

    /// One window, out of the dictionary describing it.
    ///
    /// [`None`] where the window server named no number for it or
    /// would not say where it stands, since neither the search nor
    /// the caller has any use for a window it cannot name or place.
    /// Everything else is optional here because it is optional
    /// there.
    fn read(described: &CFDictionary) -> Option<Self> {
        let DescribedWindowNumber::Numbered { window_id } = number(described) else {
            return None;
        };
        Some(Self {
            number: window_id,
            owner:  integer(described, Key::Owner).map_or(TerminalWindowOwner::Unnamed, |pid| {
                TerminalWindowOwner::Application { pid }
            }),
            name:   text(described, Key::OwnerName).map(|name| folded(&name)),
            title:  text(described, Key::Title)
                .map_or(WindowTitle::Withheld, WindowTitle::Reported),
            bounds: bounds(described)?,
            layer:  integer(described, Key::Layer).unwrap_or_default(),
        })
    }
}

impl TerminalWindowCandidate for Listed {
    fn owner(&self) -> TerminalWindowOwner { self.owner }

    fn frontmost(&self) -> bool { self.layer == 0 }
}

impl TerminalProgramWindowCandidate for Listed {
    fn names(&self) -> Vec<String> { self.name.clone().into_iter().collect() }
}

/// See [`crate::backdrop::desktop::window_titles`].
///
/// CoreGraphics rather than `ScreenCaptureKit`, for the reason
/// [`Listed::on_screen`] gives: this is called from the thread that
/// draws.
pub(in crate::backdrop::desktop) fn window_titles() -> Vec<TitledWindow> {
    terminal_windows(&Listed::on_screen())
        .windows
        .into_iter()
        .map(|window| TitledWindow {
            window_id: window.number,
            title:     window.title.clone(),
        })
        .collect()
}

/// See [`crate::backdrop::desktop::window_titled`].
///
/// CoreGraphics rather than `ScreenCaptureKit`, for the reason
/// [`Listed::on_screen`] gives.
pub(in crate::backdrop::desktop) fn window_titled(marker: &str) -> TerminalWindowSearchOutcome {
    terminal_windows(&Listed::on_screen())
        .windows
        .into_iter()
        .find(|window| match &window.title {
            WindowTitle::Reported(title) => title.contains(marker),
            WindowTitle::Withheld => false,
        })
        .map_or(TerminalWindowSearchOutcome::NotFound, |window| {
            TerminalWindowSearchOutcome::Found {
                window_id: window.number,
            }
        })
}

/// See [`crate::backdrop::desktop::window_at`].
///
/// CoreGraphics rather than `ScreenCaptureKit`, for the reason
/// [`Listed::on_screen`] gives. Nothing here filters for windows
/// that are on screen because the list asked for holds no others.
pub(in crate::backdrop::desktop) fn window_at(origin: (f64, f64)) -> TerminalWindowSearchOutcome {
    terminal_windows(&Listed::on_screen())
        .windows
        .into_iter()
        .map(|window| (window.number, away(window.bounds, origin)))
        .filter(|(_, away)| *away <= POSITION_TOLERANCE)
        .min_by(|(_, left), (_, right)| left.total_cmp(right))
        .map_or(TerminalWindowSearchOutcome::NotFound, |(window_id, _)| {
            TerminalWindowSearchOutcome::Found { window_id }
        })
}

/// How far a window's corner stands from `origin`, for
/// [`window_at`] to sort on.
///
/// Along both axes added together rather than as a diagonal. The
/// question is which of a handful of windows is nearest, and every
/// ordering that answers it agrees; a square root would only cost
/// what it does not settle.
fn away(frame: CoreGraphicsRect, origin: (f64, f64)) -> f64 {
    (frame.origin.x - origin.0).abs() + (frame.origin.y - origin.1).abs()
}

/// See [`crate::backdrop::desktop::window_frame`].
///
/// CoreGraphics rather than `ScreenCaptureKit` because of what the
/// two cost: `SCShareableContent::get` describes every window on
/// the machine and takes about seventy milliseconds, where asking
/// CoreGraphics about one window by number takes a few hundred
/// microseconds. Both report the same rectangle, so nothing is
/// given up by asking the cheaper of them.
pub(in crate::backdrop::desktop) fn window_frame(window: u32) -> Option<Frame> {
    let list = CGWindowListCopyWindowInfo(CGWindowListOption::OptionIncludingWindow, window)?;
    // `entry(&list, 0)` returns `None` if the window has been closed since the capture.
    let described = entry(&list, 0)?;
    let rect = bounds(described)?;
    Some(Frame {
        origin: (rect.origin.x, rect.origin.y),
        size:   (rect.size.width, rect.size.height),
    })
}

/// The dictionary describing one window of a `CGWindowList` answer.
///
/// # Invariants
///
/// `list` must have come from [`CGWindowListCopyWindowInfo`], whose
/// elements are Core Foundation objects the array holds a reference
/// to. Nothing weaker will do: a `CFArray` can be built with null
/// callbacks and filled with bare integers instead of objects, and
/// an array that was cannot be told from one that was not.
#[allow(
    unsafe_code,
    reason = "CoreFoundation collections have no safe binding: their \
              accessors hand back untyped pointers, and the caller \
              carries the invariant above in their place"
)]
fn entry(list: &CFArray, index: isize) -> Option<&CFDictionary> {
    if !(0..list.count()).contains(&index) {
        return None;
    }
    // SAFETY: `list.value_at_index(index)` is undefined outside the
    // array's own index space, and the check above is what holds
    // `index` inside it -- both ends of it, since a `CFIndex` is
    // signed and a negative one is as far out of the space as a
    // large one. The binding's own requirement is that the array's
    // generic match its contents, which an untyped `CFArray` has no
    // way to fail.
    let value = unsafe { list.value_at_index(index) };
    // SAFETY: `value` is a Core Foundation object by this
    // function's invariant, so a non-null pointer to one points at a
    // live `CFType`; null is what `as_ref` answers `None` for. The
    // array owns a reference to that object for as long as the array
    // is alive, and the borrow handed back is tied to `list` by
    // lifetime elision, so nothing can read it after the array has
    // gone.
    let value = unsafe { value.cast::<CFType>().as_ref() }?;
    // That `value` is a dictionary in particular is asked rather
    // than assumed: `downcast_ref` compares `CFGetTypeID` and hands
    // back `None` for anything else.
    value.downcast_ref::<CFDictionary>()
}

/// What a window's dictionary holds under `key`, or [`None`] where
/// it holds nothing under it.
///
/// A window server that will not answer a question leaves the entry
/// out rather than emptying it, so this is missing far more often
/// than it looks: [`Key::Title`] is absent for every window the
/// process has no Screen Recording permission to read.
///
/// # Invariants
///
/// `described` must be one of [`entry`]'s dictionaries, and so must
/// hold Core Foundation objects for the same reason its array does.
#[allow(
    unsafe_code,
    reason = "CoreFoundation collections have no safe binding: their \
              accessors hand back untyped pointers, and the caller \
              carries the invariant above in their place"
)]
fn value(described: &CFDictionary, key: Key) -> Option<&CFType> {
    let key = std::ptr::from_ref(key.name()).cast::<c_void>();
    // SAFETY: `described.value(key)` wants a valid key pointer,
    // which is what `key` was just made from a live constant, and a
    // dictionary whose generic matches its contents, which an
    // untyped `CFDictionary` has no way to fail. It answers null for
    // a key the dictionary does not hold.
    let value = unsafe { described.value(key) };
    // SAFETY: `value` is a Core Foundation object by this
    // function's invariant, so a non-null pointer to one points at a
    // live `CFType`, and `as_ref` answers `None` for the null.
    // `described` holds a reference to it and outlives this call.
    unsafe { value.cast::<CFType>().as_ref() }
}

/// A whole number out of a window's dictionary.
///
/// The type is asked rather than assumed, here and in every reader
/// below it: `downcast_ref` compares `CFGetTypeID` and hands back
/// `None` for anything else, so a window server answering something
/// other than what it documents costs a missing field and not a
/// misread one.
fn integer(described: &CFDictionary, key: Key) -> Option<i32> {
    value(described, key)?.downcast_ref::<CFNumber>()?.as_i32()
}

/// A string out of a window's dictionary.
fn text(described: &CFDictionary, key: Key) -> Option<String> {
    Some(
        value(described, key)?
            .downcast_ref::<CFString>()?
            .to_string(),
    )
}

/// A window's own number, out of the dictionary describing it, where
/// the window server supplied one a caller can use.
///
/// # Invariants
///
/// `described` must be one of [`entry`]'s dictionaries, and so must
/// hold Core Foundation objects for the same reason its array does.
fn number(described: &CFDictionary) -> DescribedWindowNumber {
    // `number` reads a `u32` window number that CoreGraphics reports as a
    // signed one.
    integer(described, Key::Number)
        .and_then(|number| u32::try_from(number).ok())
        .map_or(DescribedWindowNumber::Unnumbered, |window_id| {
            DescribedWindowNumber::Numbered { window_id }
        })
}

/// A window's bounds, out of the dictionary describing it.
///
/// # Invariants
///
/// `described` must be one of [`entry`]'s dictionaries, and so must
/// hold Core Foundation objects for the same reason its array does.
#[allow(
    unsafe_code,
    reason = "`CGRectMakeWithDictionaryRepresentation` has no safe \
              binding: it writes through an out pointer, which the \
              caller supplies and the SAFETY comment below covers"
)]
fn bounds(described: &CFDictionary) -> Option<CoreGraphicsRect> {
    // `Key::Bounds` is documented to be a rect in dictionary form, and asked rather
    // than assumed, because the call below is undefined on something
    // that is not a dictionary at all. Given one that is but does
    // not describe a rect, it answers false and writes nothing.
    let value = value(described, Key::Bounds)?.downcast_ref::<CFDictionary>()?;
    let mut rect = CoreGraphicsRect::default();
    // SAFETY: the `rect` out pointer is to a live, aligned, already
    // initialised `CGRect` in this frame, so it stays valid to write
    // for the whole of a call that cannot outlast the frame; and the
    // dictionary is the one type-checked just above.
    unsafe { CGRectMakeWithDictionaryRepresentation(Some(value), std::ptr::from_mut(&mut rect)) }
        .then_some(rect)
}

/// Every on-screen window belonging to this process or one of its
/// ancestors -- which is how the terminal emulator hosting this app
/// is found, since the window is the emulator's and not this
/// process's.
pub(super) fn terminal_windows<W: TerminalWindowCandidate + TerminalProgramWindowCandidate>(
    windows: &[W],
) -> TerminalWindowCandidates<'_, W> {
    let ancestors = ancestor_pids();
    // An emulator that hosts its sessions in a server process of its
    // own is nowhere in this app's parent chain: iTerm2's shell hangs
    // off `iTermServer`, and the process drawing the window is not an
    // ancestor of anything running in it. `named_emulator_windows` asks the emulator who it is
    // instead -- it says so in the environment it handed down.
    let named = named_emulator_windows(windows);
    candidate::terminal_window_candidates(
        windows,
        |pid| ancestors.contains(&pid),
        |window| {
            named
                .iter()
                .any(|named_window| std::ptr::eq(*named_window, window))
        },
    )
}

/// Every on-screen window of the terminal emulator named by
/// `TERM_PROGRAM`.
///
/// Every emulator worth the name sets this in the environment it
/// hands the shell -- `iTerm.app`, `Apple_Terminal`, `WezTerm`,
/// `ghostty` -- and it survives the walk down to this process
/// however many shells stand in between, which is exactly what the
/// parent chain does not. It names the application rather than the
/// window, so it cannot tell two windows of one emulator apart;
/// what it does rule out is choosing a window belonging to some
/// other application entirely, which is the way this went wrong.
///
/// Empty where the variable is unset, where it names nothing on
/// screen, or where this is not a terminal at all.
fn named_emulator_windows<W: TerminalProgramWindowCandidate>(windows: &[W]) -> Vec<&W> {
    let Ok(program) = env::var(TERM_PROGRAM_ENV) else {
        return Vec::new();
    };
    // `TERM_PROGRAM` names a bundle, extension and all: iTerm2 sets
    // `iTerm.app`. `folded` keeps every letter, so the extension
    // survives it as `itermapp` -- which is inside neither the
    // application's own name, `iterm2`, nor its bundle identifier,
    // `comgooglecodeiterm2`. The `names_agree` containment test then fails in
    // both directions and every window of the emulator is passed
    // over, leaving whichever application stands in front to be
    // taken for the terminal.
    let wanted = folded(program.strip_suffix(".app").unwrap_or(&program));
    if wanted.len() < EMULATOR_NAME_FLOOR {
        return Vec::new();
    }
    windows
        .iter()
        .filter(|window| {
            window
                .names()
                .iter()
                .any(|found| names_agree(&wanted, found))
        })
        .collect()
}

/// `text` with everything but its letters and digits taken out and
/// the rest put in lower case, so that the several ways one
/// emulator writes its own name can be compared.
///
/// `TERM_PROGRAM` gives `iTerm.app`, the application calls itself
/// `iTerm2` and its bundle is `com.googlecode.iterm2`; folded, all
/// three carry `iterm` -- once the `.app` extension has been taken
/// off the first, which [`named_emulator_windows`] does before
/// folding it. This keeps letters and digits, so an extension left
/// on survives as part of the name.
fn folded(text: &str) -> String {
    text.chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|character| character.to_ascii_lowercase())
        .collect()
}

/// Whether two folded names are the same emulator.
///
/// Either may be the longer: `appleterminal` holds `terminal`, and
/// `comgooglecodeiterm2` holds `iterm`. Both are held to
/// [`EMULATOR_NAME_FLOOR`] first, because a name of two or three
/// letters is inside half the bundle identifiers on the machine.
fn names_agree(wanted: &str, found: &str) -> bool {
    found.len() >= EMULATOR_NAME_FLOOR && (found.contains(wanted) || wanted.contains(found))
}

/// The one of the emulator's windows this app is drawn in.
///
/// An emulator commonly has several windows open, and every one of
/// them answers to the same application. Size is what tells them
/// apart: `TIOCGWINSZ` reports this tty's own text area, and the
/// window that area belongs to is the one whose frame it very
/// nearly fills -- short by a title bar and whatever padding the
/// emulator draws, and no more.
///
/// Neither of the obvious answers works. The biggest window is
/// whichever one happens to be biggest, and `is_active` is set on
/// every window of the active application rather than on the key
/// one. Both pick a sibling window as readily as this one, and what
/// arrives then is the desktop behind something else.
pub(super) fn frontmost_window<'a>(
    windows: &'a [&'a Listed],
    displays: &[Display],
    text_pixels: (u16, u16),
) -> Option<&'a Listed> {
    // `score` compares each `Listed` candidate against the `backing_scale` of the display it
    // stands on rather than one scale for all of them, since a
    // machine with a Retina panel and an external monitor is
    // carrying both at once.
    let score = |window: &Listed| {
        let frame = window.bounds;
        let scale = display_under(displays, frame).map_or(1, backing_scale);
        mismatch(frame, text_pixels, scale)
    };
    windows
        .iter()
        .min_by(|left, right| score(left).total_cmp(&score(right)))
        .copied()
}

/// How far a window's frame is from holding a text area of
/// `text_pixels`, for [`frontmost_window`] to sort on.
///
/// A frame narrower or shorter than the text area cannot be the one
/// holding it, so falling short counts double and a frame that is
/// merely larger by a title bar stays the nearest.
///
/// The reported area is divided by the `scale` of the display this
/// candidate stands on before anything is compared. Scored as
/// reported, a text area on a Retina panel reads as twice the
/// window holding it, and every candidate on that panel is judged
/// short by the same doubled amount -- which is no ordering at all.
fn mismatch(frame: CoreGraphicsRect, text_pixels: (u16, u16), scale: u32) -> f64 {
    let scale = f64::from(scale);
    let width = f64::from(text_pixels.0) / scale;
    let height = f64::from(text_pixels.1) / scale;
    let axis = |frame: f64, text: f64| {
        let difference = frame - text;
        if difference < 0.0 {
            -difference * 2.0
        } else {
            difference
        }
    };
    axis(frame.size.width, width) + axis(frame.size.height, height)
}

/// This process's pid and every pid above it, as the window server
/// numbers them.
fn ancestor_pids() -> HashSet<i32> {
    let mut system = System::new();
    system.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::nothing());
    let mut pids = HashSet::new();
    let mut current = Pid::from_u32(std::process::id());
    // A cycle in the parent chain would spin here forever, so a pid
    // already in `pids` ends the walk as surely as a missing parent does.
    while let Ok(pid) = i32::try_from(current.as_u32()) {
        if !pids.insert(pid) {
            break;
        }
        // sysinfo leaves the parent unset for a process it cannot
        // read, and `/usr/bin/login` between the emulator and the
        // shell is exactly that -- so `process::kernel_parent` answers where the
        // scan will not, or the walk never reaches the emulator
        // whose window this capture is about.
        match system
            .process(current)
            .and_then(sysinfo::Process::parent)
            .or_else(|| process::kernel_parent(current))
        {
            Some(parent) => current = parent,
            None => break,
        }
    }
    pids
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    /// The three names iTerm2 answers to, as
    /// [`named_emulator_windows`](super::named_emulator_windows)
    /// meets them: `TERM_PROGRAM`, the application's own name, and
    /// its bundle identifier.
    const TERM_PROGRAM: &str = "iTerm.app";
    /// What `SCRunningApplication` calls iTerm2.
    const APPLICATION: &str = "iTerm2";
    /// iTerm2's bundle identifier.
    const BUNDLE: &str = "com.googlecode.iterm2";

    #[test]
    fn folding_keeps_an_extension_that_is_left_on() {
        assert_eq!(super::folded(TERM_PROGRAM), "itermapp");
    }

    #[test]
    fn folding_a_stripped_term_program_gives_the_bare_name() {
        let stripped = TERM_PROGRAM.strip_suffix(".app").expect("names a bundle");
        assert_eq!(super::folded(stripped), "iterm");
    }

    #[test]
    fn an_extension_left_on_agrees_with_neither_name() {
        let wanted = super::folded(TERM_PROGRAM);
        assert!(!super::names_agree(&wanted, &super::folded(APPLICATION)));
        assert!(!super::names_agree(&wanted, &super::folded(BUNDLE)));
    }

    #[test]
    fn a_stripped_term_program_agrees_with_both_names() {
        let stripped = TERM_PROGRAM.strip_suffix(".app").expect("names a bundle");
        let wanted = super::folded(stripped);
        assert!(super::names_agree(&wanted, &super::folded(APPLICATION)));
        assert!(super::names_agree(&wanted, &super::folded(BUNDLE)));
    }

    #[test]
    fn an_emulator_naming_no_bundle_is_unaffected() {
        for (program, application) in [
            ("Apple_Terminal", "Terminal"),
            ("WezTerm", "WezTerm"),
            ("ghostty", "Ghostty"),
        ] {
            let stripped = program.strip_suffix(".app").unwrap_or(program);
            assert!(
                super::names_agree(&super::folded(stripped), &super::folded(application)),
                "{program} should agree with {application}"
            );
        }
    }
}
