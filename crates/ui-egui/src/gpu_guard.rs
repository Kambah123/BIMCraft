//! A watchdog for the 3D view on the graphics card in the browser. A browser runs the card's work on the page's one
//! thread, so a card that takes too long (shaders compiling for many seconds, an upload or a picture that stalls) holds
//! up the whole tab. Each step is timed: one slower than `SLOW_MS` puts the 3D view on the processor at once, and the
//! browser remembers it (`localStorage` key `campusbim.gpu3d`). A try that never came back (the tab froze and was closed)
//! is remembered too: "trying" is written before the card is made and "ok" only after a few good pictures, so the next
//! load finds "trying" and starts on the processor. `?gpu3d=retry` forgets it. On the desktop the guard does nothing.

/// The longest one step on the card may hold up the page.
pub(crate) const SLOW_MS: f64 = 2000.0;
/// Frames of the 3D view shown on the card, after a whole picture drawn in time, before the card is trusted here.
const GOOD_FRAMES: u32 = 10;

#[derive(Default)]
pub(crate) struct Guard {
    /// The browser's: the desktop's guard never bars the card.
    active: bool,
    /// A whole picture was drawn in time since the card was made.
    drawn: bool,
    good: u32,
    trusted: bool,
}

impl Guard {
    /// The guard for this build: on in the browser, off on the desktop.
    pub(crate) fn for_build() -> Guard {
        Guard { active: cfg!(target_arch = "wasm32"), ..Guard::default() }
    }

    #[cfg(test)]
    pub(crate) fn on() -> Guard {
        Guard { active: true, ..Guard::default() }
    }

    /// Before the card is made: why it may not be, if an earlier try in this browser froze or was too slow.
    pub(crate) fn barred(&self) -> Option<String> {
        if !self.active {
            return None;
        }
        if store::retry_asked() {
            store::write("");
            return None;
        }
        let kept = store::read()?;
        if kept == "trying" {
            Some("the graphics card didn't finish starting the 3D view the last time".into())
        } else {
            kept.strip_prefix("failed ").map(|why| format!("the graphics card was too slow here before ({why})"))
        }
    }

    /// The card is about to be made: remembered until it has drawn a few pictures in time.
    pub(crate) fn arm(&mut self) {
        if self.active {
            store::write("trying");
            (self.drawn, self.good, self.trusted) = (false, 0, false);
        }
    }

    /// A step on the card took `ms`; `whole` when it drew the whole picture. Too slow: why, and the card is barred here from
    /// now on.
    pub(crate) fn step(&mut self, what: &str, ms: f64, whole: bool) -> Result<(), String> {
        if !self.active {
            return Ok(());
        }
        if ms > SLOW_MS {
            let why = format!("{what} took {ms:.0} ms");
            store::write(&format!("failed {why}"));
            return Err(why);
        }
        self.drawn |= whole;
        Ok(())
    }

    /// A frame of the 3D view on the card went by: after a whole picture and a few frames more, the card is trusted here
    /// (a card that hangs the page stops the frames before that, and the next load starts on the processor).
    /// True while it is still watching, so that the caller draws another frame soon.
    pub(crate) fn frame(&mut self) -> bool {
        if self.active && self.drawn && !self.trusted {
            self.good += 1;
            if self.good >= GOOD_FRAMES {
                self.trusted = true;
                store::write("ok");
            }
        }
        self.active && self.drawn && !self.trusted
    }
}

#[cfg(target_arch = "wasm32")]
mod store {
    const KEY: &str = "campusbim.gpu3d";
    fn storage() -> Option<web_sys::Storage> {
        web_sys::window()?.local_storage().ok().flatten()
    }
    pub(super) fn read() -> Option<String> {
        storage()?.get_item(KEY).ok().flatten()
    }
    pub(super) fn write(value: &str) {
        if let Some(s) = storage() {
            let _ = if value.is_empty() { s.remove_item(KEY) } else { s.set_item(KEY, value) };
        }
    }
    pub(super) fn retry_asked() -> bool {
        web_sys::window().and_then(|w| w.location().search().ok()).is_some_and(|q| q.contains("gpu3d=retry"))
    }
}

/// The desktop keeps nothing; tests keep it per thread.
#[cfg(not(target_arch = "wasm32"))]
mod store {
    use std::cell::RefCell;
    thread_local!(static KEPT: RefCell<Option<String>> = const { RefCell::new(None) });
    pub(super) fn read() -> Option<String> {
        KEPT.with_borrow(Clone::clone)
    }
    pub(super) fn write(value: &str) {
        KEPT.with_borrow_mut(|k| *k = (!value.is_empty()).then(|| value.to_string()));
    }
    pub(super) fn retry_asked() -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_desktop_never_bars_the_card() {
        let mut g = Guard::default();
        g.arm();
        assert!(g.step("x", 60_000.0, true).is_ok());
        assert!(g.barred().is_none());
    }

    #[test]
    fn a_try_that_never_finished_bars_the_next_load() {
        store::write("");
        let mut g = Guard::on();
        assert!(g.barred().is_none());
        g.arm();
        // The tab froze here: the next load reads what was left.
        assert!(Guard::on().barred().is_some_and(|w| w.contains("didn't finish")));
        store::write("");
    }

    #[test]
    fn a_slow_step_bars_the_card_and_good_pictures_trust_it() {
        store::write("");
        let mut g = Guard::on();
        g.arm();
        assert!(g.step("a 3D picture", 20.0, true).is_ok());
        assert!(Guard::on().barred().is_some(), "not trusted on the first picture alone");
        for _ in 0..GOOD_FRAMES {
            g.frame();
        }
        assert!(!g.frame());
        assert!(Guard::on().barred().is_none());
        assert!(g.step("starting the card", SLOW_MS + 1.0, false).is_err());
        assert!(Guard::on().barred().is_some_and(|w| w.contains("starting the card")));
        store::write("");
    }
}
