//! OS layer: a window that shows a CPU framebuffer, and keyboard state. Kept tiny on purpose; the
//! engine core never depends on it, so everything also runs headless (tests, servers, CI).

/// Keyboard state, indexed by Windows virtual-key code (letters are their ASCII uppercase).
#[derive(Clone)]
pub struct Input {
    pub down: [bool; 256],
    /// Went down since the previous `pump`.
    pub pressed: [bool; 256],
}

impl Default for Input {
    fn default() -> Input {
        Input { down: [false; 256], pressed: [false; 256] }
    }
}

impl Input {
    pub const LEFT: u8 = 0x25;
    pub const UP: u8 = 0x26;
    pub const RIGHT: u8 = 0x27;
    pub const DOWN: u8 = 0x28;
    pub const SPACE: u8 = 0x20;
    pub const ESCAPE: u8 = 0x1B;

    pub fn is_down(&self, key: u8) -> bool {
        self.down[key as usize]
    }
    pub fn was_pressed(&self, key: u8) -> bool {
        self.pressed[key as usize]
    }
}

#[cfg(windows)]
mod win32;
#[cfg(windows)]
pub use win32::Window;

#[cfg(not(windows))]
pub struct Window;

#[cfg(not(windows))]
impl Window {
    pub fn open(_title: &str, _width: usize, _height: usize) -> Result<Window, String> {
        Err("windowed mode is only implemented for Windows; run with --headless".into())
    }
    pub fn pump(&mut self, _input: &mut Input) -> bool {
        false
    }
    pub fn present(&mut self, _pixels: &[u32], _width: usize, _height: usize) {}
    pub fn set_title(&mut self, _title: &str) {}
}
