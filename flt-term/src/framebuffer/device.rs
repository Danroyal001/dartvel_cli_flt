//! Opening and mapping a Linux framebuffer device, and keeping the console's
//! text off it.

use std::fs::{File, OpenOptions};
use std::io;
use std::os::fd::AsRawFd;

use super::{Channel, Layout};

const FBIOGET_VSCREENINFO: libc::c_ulong = 0x4600;
const FBIOGET_FSCREENINFO: libc::c_ulong = 0x4602;
const KDSETMODE: libc::c_ulong = 0x4B3A;
const KD_TEXT: libc::c_ulong = 0;
const KD_GRAPHICS: libc::c_ulong = 1;

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct Bitfield {
    offset: u32,
    length: u32,
    msb_right: u32,
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct VarScreenInfo {
    xres: u32,
    yres: u32,
    xres_virtual: u32,
    yres_virtual: u32,
    xoffset: u32,
    yoffset: u32,
    bits_per_pixel: u32,
    grayscale: u32,
    red: Bitfield,
    green: Bitfield,
    blue: Bitfield,
    transp: Bitfield,
    nonstd: u32,
    activate: u32,
    height: u32,
    width: u32,
    accel_flags: u32,
    pixclock: u32,
    left_margin: u32,
    right_margin: u32,
    upper_margin: u32,
    lower_margin: u32,
    hsync_len: u32,
    vsync_len: u32,
    sync: u32,
    vmode: u32,
    rotate: u32,
    colorspace: u32,
    reserved: [u32; 4],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct FixScreenInfo {
    id: [u8; 16],
    smem_start: libc::c_ulong,
    smem_len: u32,
    kind: u32,
    type_aux: u32,
    visual: u32,
    xpanstep: u16,
    ypanstep: u16,
    ywrapstep: u16,
    line_length: u32,
    mmio_start: libc::c_ulong,
    mmio_len: u32,
    accel: u32,
    capabilities: u16,
    reserved: [u16; 2],
}

/// The framebuffer device path: `$FRAMEBUFFER`, else `/dev/fb0`.
pub fn path() -> String {
    std::env::var("FRAMEBUFFER").ok().filter(|path| !path.is_empty()).unwrap_or_else(|| "/dev/fb0".into())
}

/// Whether the framebuffer can be opened for writing, without keeping it.
pub fn writable() -> bool {
    OpenOptions::new().read(true).write(true).open(path()).is_ok()
}

/// A mapped framebuffer. Unmapped on drop.
pub struct Framebuffer {
    _file: File,
    memory: *mut u8,
    length: usize,
    pub layout: Layout,
}

// The mapping belongs to this value alone and is only reached through it.
unsafe impl Send for Framebuffer {}

impl Framebuffer {
    pub fn open() -> io::Result<Framebuffer> {
        let file = OpenOptions::new().read(true).write(true).open(path())?;
        let fd = file.as_raw_fd();
        let mut var = VarScreenInfo::default();
        // SAFETY: FBIOGET_VSCREENINFO fills a struct fb_var_screeninfo, which
        // VarScreenInfo mirrors field for field.
        if unsafe { libc::ioctl(fd, FBIOGET_VSCREENINFO as _, &mut var) } < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: as above, for struct fb_fix_screeninfo; zeroes are a valid
        // value of every field.
        let mut fix: FixScreenInfo = unsafe { std::mem::zeroed() };
        if unsafe { libc::ioctl(fd, FBIOGET_FSCREENINFO as _, &mut fix) } < 0 {
            return Err(io::Error::last_os_error());
        }
        let layout = Layout {
            width: var.xres,
            height: var.yres,
            line_length: fix.line_length as usize,
            bits_per_pixel: var.bits_per_pixel,
            red: Channel { offset: var.red.offset, length: var.red.length },
            green: Channel { offset: var.green.offset, length: var.green.length },
            blue: Channel { offset: var.blue.offset, length: var.blue.length },
        };
        if !layout.supported() {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                format!("framebuffer pixel layout not supported: {} bits per pixel", layout.bits_per_pixel),
            ));
        }
        let length = (fix.smem_len as usize).max(layout.line_length * layout.height as usize);
        // SAFETY: mapping the device's own memory, length from the device;
        // checked for failure below and unmapped exactly once in Drop.
        let memory = unsafe {
            libc::mmap(std::ptr::null_mut(), length, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_SHARED, fd, 0)
        };
        if memory == libc::MAP_FAILED {
            return Err(io::Error::last_os_error());
        }
        Ok(Framebuffer { _file: file, memory: memory as *mut u8, length, layout })
    }

    /// The mapped pixels.
    pub fn memory(&mut self) -> &mut [u8] {
        // SAFETY: the mapping is `length` bytes and lives as long as self.
        unsafe { std::slice::from_raw_parts_mut(self.memory, self.length) }
    }
}

impl Drop for Framebuffer {
    fn drop(&mut self) {
        // SAFETY: the pointer and length are the ones mmap returned.
        unsafe { libc::munmap(self.memory as *mut libc::c_void, self.length) };
    }
}

/// Puts the console on `tty_fd` into graphics mode, so it stops drawing text
/// and its cursor over the framebuffer. Paired with [text_mode]; the runner
/// restores text mode on exit, on a signal and on a crash, because a console
/// left in graphics mode looks dead.
pub fn graphics_mode(tty_fd: i32) -> io::Result<()> {
    // SAFETY: KDSETMODE takes an integer argument.
    if unsafe { libc::ioctl(tty_fd, KDSETMODE as _, KD_GRAPHICS) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Async-signal-safe: one ioctl.
pub fn text_mode(tty_fd: i32) {
    // SAFETY: as above.
    unsafe { libc::ioctl(tty_fd, KDSETMODE as _, KD_TEXT) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_structs_match_the_kernel_sizes() {
        // linux/fb.h on 64-bit: fb_var_screeninfo is 160 bytes,
        // fb_fix_screeninfo 80.
        assert_eq!(std::mem::size_of::<VarScreenInfo>(), 160);
        #[cfg(target_pointer_width = "64")]
        assert_eq!(std::mem::size_of::<FixScreenInfo>(), 80);
    }
}
