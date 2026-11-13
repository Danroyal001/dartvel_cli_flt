//! Raw mode, window size and crash-safe restore for a Unix terminal.

use std::io;
use std::os::fd::RawFd;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::OnceLock;
use std::time::Duration;

/// The terminal device the runner draws on and reads from.
pub struct Tty {
    pub fd: RawFd,
    original: Option<libc::termios>,
    owned: bool,
}

/// The terminal's size in cells and, when it says, pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowSize {
    pub columns: u16,
    pub rows: u16,
    pub width_pixels: u16,
    pub height_pixels: u16,
}

impl Tty {
    /// `/dev/tty`, the controlling terminal, wherever stdin and stdout point.
    pub fn open() -> io::Result<Tty> {
        let path = c"/dev/tty";
        // SAFETY: a NUL-terminated path and plain flags.
        let fd = unsafe { libc::open(path.as_ptr(), libc::O_RDWR | libc::O_NOCTTY | libc::O_CLOEXEC) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Tty { fd, original: None, owned: true })
    }

    /// An already open terminal descriptor, not closed on drop.
    pub fn from_fd(fd: RawFd) -> Tty {
        Tty { fd, original: None, owned: false }
    }

    /// Raw mode: no echo, no line buffering, no signals from keys (Ctrl+C
    /// arrives as a key the runner handles), no output processing.
    pub fn enter_raw(&mut self) -> io::Result<()> {
        // SAFETY: termios is plain data; tcgetattr fills it.
        let mut attributes: libc::termios = unsafe { std::mem::zeroed() };
        if unsafe { libc::tcgetattr(self.fd, &mut attributes) } < 0 {
            return Err(io::Error::last_os_error());
        }
        self.original.get_or_insert(attributes);
        let mut raw = attributes;
        // SAFETY: cfmakeraw edits the struct in place.
        unsafe { libc::cfmakeraw(&mut raw) };
        raw.c_cc[libc::VMIN] = 1;
        raw.c_cc[libc::VTIME] = 0;
        if unsafe { libc::tcsetattr(self.fd, libc::TCSANOW, &raw) } < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    /// The attributes before [Tty::enter_raw], for [arm_restore].
    pub fn original(&self) -> Option<libc::termios> {
        self.original
    }

    pub fn size(&self) -> io::Result<WindowSize> {
        // SAFETY: winsize is plain data; TIOCGWINSZ fills it.
        let mut size: libc::winsize = unsafe { std::mem::zeroed() };
        if unsafe { libc::ioctl(self.fd, libc::TIOCGWINSZ, &mut size) } < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(WindowSize { columns: size.ws_col, rows: size.ws_row, width_pixels: size.ws_xpixel, height_pixels: size.ws_ypixel })
    }

    /// Writes everything. A blocking write is how output backlog shows: on a
    /// Linux pty TIOCOUTQ reads 0 whatever is queued, so the runner measures
    /// how long this takes rather than asking how much is waiting.
    pub fn write_all(&self, mut bytes: &[u8]) -> io::Result<()> {
        while !bytes.is_empty() {
            // SAFETY: writing from a valid slice.
            let written = unsafe { libc::write(self.fd, bytes.as_ptr() as *const libc::c_void, bytes.len()) };
            if written < 0 {
                let error = io::Error::last_os_error();
                if error.kind() == io::ErrorKind::Interrupted || error.kind() == io::ErrorKind::WouldBlock {
                    continue;
                }
                return Err(error);
            }
            bytes = &bytes[written as usize..];
        }
        Ok(())
    }

    /// Reads what is available, waiting at most `timeout` (None: forever).
    /// Ok(0) means the wait ended with nothing to read.
    pub fn read_timeout(&self, buffer: &mut [u8], timeout: Option<Duration>) -> io::Result<usize> {
        if !wait_readable(&[self.fd], timeout)?.contains(&self.fd) {
            return Ok(0);
        }
        // SAFETY: reading into a valid slice.
        let read = unsafe { libc::read(self.fd, buffer.as_mut_ptr() as *mut libc::c_void, buffer.len()) };
        if read < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                return Ok(0);
            }
            return Err(error);
        }
        Ok(read as usize)
    }
}

impl Drop for Tty {
    fn drop(&mut self) {
        if self.owned {
            // SAFETY: our own descriptor, closed once.
            unsafe { libc::close(self.fd) };
        }
    }
}

/// The descriptors among `fds` that are readable within `timeout`.
pub fn wait_readable(fds: &[RawFd], timeout: Option<Duration>) -> io::Result<Vec<RawFd>> {
    let mut polls: Vec<libc::pollfd> = fds.iter().map(|&fd| libc::pollfd { fd, events: libc::POLLIN, revents: 0 }).collect();
    let millis = timeout.map_or(-1, |timeout| timeout.as_millis().min(i32::MAX as u128) as i32);
    // SAFETY: a valid array of pollfd.
    let ready = unsafe { libc::poll(polls.as_mut_ptr(), polls.len() as libc::nfds_t, millis) };
    if ready < 0 {
        let error = io::Error::last_os_error();
        if error.kind() == io::ErrorKind::Interrupted {
            return Ok(Vec::new());
        }
        return Err(error);
    }
    Ok(polls.iter().filter(|poll| poll.revents & (libc::POLLIN | libc::POLLHUP) != 0).map(|poll| poll.fd).collect())
}

// What a crash or signal handler needs, reachable without allocating or
// locking. Set once by `arm_restore`.
static RESTORE_FD: AtomicI32 = AtomicI32::new(-1);
static CONSOLE_FD: AtomicI32 = AtomicI32::new(-1);
static RESTORE_TERMIOS: OnceLock<libc::termios> = OnceLock::new();
static RESTORE_BYTES: OnceLock<Vec<u8>> = OnceLock::new();
static RESTORED: AtomicBool = AtomicBool::new(false);
static WINCH_WRITE: AtomicI32 = AtomicI32::new(-1);

/// Records how to give the terminal back: write `bytes` to `fd`, put back
/// `termios`, and switch `console_fd` (a VT in framebuffer mode) back to text.
pub fn arm_restore(fd: RawFd, termios: Option<libc::termios>, bytes: Vec<u8>, console_fd: Option<RawFd>) {
    if let Some(termios) = termios {
        let _ = RESTORE_TERMIOS.set(termios);
    }
    let _ = RESTORE_BYTES.set(bytes);
    CONSOLE_FD.store(console_fd.unwrap_or(-1), Ordering::SeqCst);
    RESTORE_FD.store(fd, Ordering::SeqCst);
    RESTORED.store(false, Ordering::SeqCst);
}

/// Gives the terminal back, once. Async-signal-safe: only write(2),
/// tcsetattr(3) and ioctl(2).
pub fn restore_now() {
    if RESTORED.swap(true, Ordering::SeqCst) {
        return;
    }
    let fd = RESTORE_FD.load(Ordering::SeqCst);
    if fd < 0 {
        return;
    }
    let console = CONSOLE_FD.load(Ordering::SeqCst);
    if console >= 0 {
        crate::framebuffer::device_text_mode(console);
    }
    if let Some(bytes) = RESTORE_BYTES.get() {
        let mut rest: &[u8] = bytes;
        while !rest.is_empty() {
            // SAFETY: writing from a static slice.
            let written = unsafe { libc::write(fd, rest.as_ptr() as *const libc::c_void, rest.len()) };
            if written <= 0 {
                break;
            }
            rest = &rest[written as usize..];
        }
    }
    if let Some(termios) = RESTORE_TERMIOS.get() {
        // SAFETY: restoring attributes read earlier from the same terminal.
        unsafe { libc::tcsetattr(fd, libc::TCSANOW, termios) };
    }
}

extern "C" fn on_fatal_signal(signal: libc::c_int) {
    restore_now();
    // SA_RESETHAND put the default action back; raising again dies with the
    // original signal, so the exit status still says what happened.
    // SAFETY: raise is async-signal-safe.
    unsafe { libc::raise(signal) };
}

extern "C" fn on_window_change(_: libc::c_int) {
    let fd = WINCH_WRITE.load(Ordering::SeqCst);
    if fd >= 0 {
        let byte = 1u8;
        // SAFETY: one byte from the stack into a non-blocking pipe.
        unsafe { libc::write(fd, &byte as *const u8 as *const libc::c_void, 1) };
    }
}

/// Restores the terminal on a panic, on SIGTERM/SIGHUP/SIGQUIT and on the
/// fatal signals an engine crash raises, then lets the process die as it
/// would have.
pub fn install_crash_handlers() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore_now();
        previous(info);
    }));
    for signal in [libc::SIGTERM, libc::SIGHUP, libc::SIGQUIT, libc::SIGSEGV, libc::SIGBUS, libc::SIGILL, libc::SIGFPE, libc::SIGABRT] {
        install(signal, on_fatal_signal as extern "C" fn(libc::c_int) as usize, libc::SA_RESETHAND);
    }
}

fn install(signal: libc::c_int, handler: usize, flags: libc::c_int) {
    // SAFETY: a zeroed sigaction with a handler and an empty mask is valid.
    unsafe {
        let mut action: libc::sigaction = std::mem::zeroed();
        action.sa_sigaction = handler;
        action.sa_flags = flags;
        libc::sigemptyset(&mut action.sa_mask);
        libc::sigaction(signal, &action, std::ptr::null_mut());
    }
}

/// A descriptor that becomes readable when the terminal is resized
/// (SIGWINCH), for the input thread to poll beside the terminal.
pub fn window_changes() -> io::Result<RawFd> {
    let mut fds = [0 as libc::c_int; 2];
    // SAFETY: pipe2 fills two descriptors.
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_NONBLOCK | libc::O_CLOEXEC) } < 0 {
        return Err(io::Error::last_os_error());
    }
    WINCH_WRITE.store(fds[1], Ordering::SeqCst);
    install(libc::SIGWINCH, on_window_change as extern "C" fn(libc::c_int) as usize, libc::SA_RESTART);
    Ok(fds[0])
}

/// Empties the resize pipe after a wake-up.
pub fn drain(fd: RawFd) {
    let mut sink = [0u8; 64];
    // SAFETY: reading into a stack buffer from a non-blocking pipe.
    while unsafe { libc::read(fd, sink.as_mut_ptr() as *mut libc::c_void, sink.len()) } > 0 {}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pty() -> (RawFd, RawFd) {
        let (mut master, mut slave) = (0, 0);
        // SAFETY: openpty fills two descriptors.
        let result = unsafe { libc::openpty(&mut master, &mut slave, std::ptr::null_mut(), std::ptr::null(), std::ptr::null()) };
        assert_eq!(result, 0);
        (master, slave)
    }

    fn echo(fd: RawFd) -> bool {
        let mut attributes: libc::termios = unsafe { std::mem::zeroed() };
        unsafe { libc::tcgetattr(fd, &mut attributes) };
        attributes.c_lflag & libc::ECHO != 0
    }

    #[test]
    fn raw_mode_is_entered_and_the_restore_gives_the_terminal_back_once() {
        let (master, slave) = pty();
        let mut tty = Tty::from_fd(slave);
        assert!(echo(slave));
        tty.enter_raw().unwrap();
        assert!(!echo(slave));
        arm_restore(slave, tty.original(), b"\x1b[?1049l".to_vec(), None);
        restore_now();
        restore_now(); // the second call is a no-op
        assert!(echo(slave));
        let mut read = [0u8; 64];
        let count = unsafe { libc::read(master, read.as_mut_ptr() as *mut libc::c_void, read.len()) };
        assert_eq!(&read[..count as usize], b"\x1b[?1049l", "written exactly once");
    }

    #[test]
    fn the_window_size_is_read_from_the_terminal() {
        let (_master, slave) = pty();
        let size = libc::winsize { ws_row: 24, ws_col: 80, ws_xpixel: 720, ws_ypixel: 432 };
        unsafe { libc::ioctl(slave, libc::TIOCSWINSZ, &size) };
        let tty = Tty::from_fd(slave);
        assert_eq!(tty.size().unwrap(), WindowSize { columns: 80, rows: 24, width_pixels: 720, height_pixels: 432 });
    }
}
