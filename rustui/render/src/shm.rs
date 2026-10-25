//! The shared-memory file behind an image layer. The app writes premultiplied RGBA8 pixels into it (`width * 4`
//! bytes per row); the compositor maps it read-only and uploads the part the app says changed.

use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::Read;
use std::os::fd::AsRawFd;
use std::os::unix::fs::{FileExt, OpenOptionsExt};
use std::path::Path;

pub struct ImageFile {
    file: File,
    ptr: *const u8,
    len: usize,
    width: u32,
}

// SAFETY: the mapping is read-only, is unmapped only when the struct is dropped, and readers never write.
unsafe impl Send for ImageFile {}
unsafe impl Sync for ImageFile {}

impl fmt::Debug for ImageFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ImageFile({} bytes)", self.len)
    }
}

impl ImageFile {
    /// Maps the file of a `width` × `height` layer. Refuses anything but a regular file at least that long: the
    /// path is derived from a layer id the app chose, and a short file would fault the compositor on first read.
    pub fn open(path: &Path, width: u32, height: u32) -> Result<Self, String> {
        let shown = path.display();
        let wanted = u64::from(width) * u64::from(height) * 4;
        // O_NONBLOCK so that a FIFO put in the way cannot hang the command thread; O_NOFOLLOW so that a symlink
        // cannot point the compositor at some other file.
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(path)
            .map_err(|e| format!("cannot open {shown}: {e}"))?;
        let meta = file
            .metadata()
            .map_err(|e| format!("cannot stat {shown}: {e}"))?;
        if !meta.is_file() {
            return Err(format!("{shown} is not a regular file"));
        }
        if meta.len() < wanted {
            return Err(format!(
                "{shown} holds {} bytes but a {width}x{height} layer needs {wanted}",
                meta.len()
            ));
        }
        let len = usize::try_from(wanted).map_err(|_| format!("{shown} is too large to map"))?;
        // SAFETY: a fresh read-only shared mapping of a file at least `len` bytes long; unmapped in `drop`.
        let ptr = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                len,
                libc::PROT_READ,
                libc::MAP_SHARED,
                file.as_raw_fd(),
                0,
            )
        };
        if ptr == libc::MAP_FAILED {
            return Err(format!(
                "cannot map {shown}: {}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(Self {
            file,
            ptr: ptr as *const u8,
            len,
            width,
        })
    }

    /// The address of pixel (`x`, `y`), for an upload that reads rows of `width * 4` bytes from there. `None` when
    /// the app has cut the file shorter than the mapping since it was opened: touching the missing pages would
    /// raise SIGBUS. The caller keeps (`x`, `y`) and the rows it reads inside the layer.
    pub fn pixel(&self, x: u32, y: u32) -> Option<*const u8> {
        let intact = self.file.metadata().ok()?.len() >= self.len as u64;
        // SAFETY: (x, y) is inside the layer by the caller's contract, so the offset stays inside the mapping.
        intact.then(|| unsafe {
            self.ptr
                .add((y as usize * self.width as usize + x as usize) * 4)
        })
    }
}

impl ImageFile {
    /// Lets go of the pages of the mapping that are mapped, without touching the file: what the compositor has read of a picture is on
    /// the GPU, and the pages stay the app's. The resident size of the compositor counts the mapped ones, whatever has been done
    /// with what they held (1.8 MB for a wallpaper's picture, which is a texture as well); a page that is read again is mapped again.
    pub fn release(&self) {
        // SAFETY: the mapping made in `open`, read-only and shared: dropping its page-table entries loses nothing, the file holds
        // the pixels.
        unsafe { libc::madvise(self.ptr as *mut libc::c_void, self.len, libc::MADV_DONTNEED) };
    }
}

impl Drop for ImageFile {
    fn drop(&mut self) {
        // SAFETY: exactly the mapping created in `open`.
        unsafe { libc::munmap(self.ptr as *mut libc::c_void, self.len) };
    }
}

/// Reads the whole of a shared-memory file that is at most `max` bytes (a mesh's geometry). Like [`ImageFile::open`]
/// it takes only a regular file it is not led to by a symlink, and it refuses a file over the limit without reading
/// any of it, so the size the app chose cannot use the device's memory up.
pub fn read_limited(path: &Path, max: u64) -> Result<Vec<u8>, String> {
    let shown = path.display();
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(|e| format!("cannot open {shown}: {e}"))?;
    let meta = file
        .metadata()
        .map_err(|e| format!("cannot stat {shown}: {e}"))?;
    if !meta.is_file() {
        return Err(format!("{shown} is not a regular file"));
    }
    if meta.len() > max {
        return Err(format!(
            "{shown} holds {} bytes; at most {max} are allowed",
            meta.len()
        ));
    }
    let mut bytes = Vec::with_capacity(meta.len() as usize);
    // The app may still be appending: never take more than the limit and one byte to notice it.
    (&mut file)
        .take(max + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("cannot read {shown}: {e}"))?;
    if bytes.len() as u64 > max {
        return Err(format!("{shown} grew past the {max} bytes allowed"));
    }
    Ok(bytes)
}

/// Reads the first `len` bytes of an atlas staging file (one upload's coverage bytes) into `out`, which holds exactly
/// them afterwards: `out` is the render thread's one buffer, reused, so it grows to the largest chunk and no further.
/// Like [`ImageFile::open`] it takes only a regular file it is not led to by a symlink, without waiting on anything the
/// app put in the way of the path (O_NONBLOCK), and it never maps the file: the app can cut it short at any moment, and
/// the pages of a mapping that is no longer there are a SIGBUS, where the same cut makes a read stop short, which is an
/// error here and nothing worse. A file longer than `len` is read as far as `len`: a slot is reused for chunks of every
/// size, and what follows the chunk is an older one's.
pub fn read_staged(path: &Path, len: usize, out: &mut Vec<u8>) -> Result<(), String> {
    let shown = path.display();
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(|e| format!("cannot open {shown}: {e}"))?;
    let meta = file
        .metadata()
        .map_err(|e| format!("cannot stat {shown}: {e}"))?;
    if !meta.is_file() {
        return Err(format!("{shown} is not a regular file"));
    }
    if meta.len() < len as u64 {
        return Err(format!(
            "{shown} holds {} bytes but the upload needs {len}",
            meta.len()
        ));
    }
    out.clear();
    out.resize(len, 0);
    // The file may have been cut since the length was looked at: the read then stops short, and says so.
    file.read_exact_at(out, 0)
        .map_err(|e| format!("cannot read {len} bytes of {shown}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;
    use std::os::unix::fs::symlink;

    fn write_layer(dir: &TempDir, name: &str, width: u32, height: u32) -> std::path::PathBuf {
        let path = dir.path().join(name);
        let pixels: Vec<u8> = (0..width * height * 4).map(|i| (i % 251) as u8).collect();
        std::fs::write(&path, pixels).unwrap();
        path
    }

    #[test]
    fn a_pixel_address_reads_what_the_app_wrote() {
        let dir = TempDir::new("shm-read");
        let path = write_layer(&dir, "layer-1.rgba", 5, 3);
        let image = ImageFile::open(&path, 5, 3).unwrap();
        let p = image.pixel(2, 1).unwrap();
        // Pixel (2, 1) starts (1 * 5 + 2) * 4 = 28 bytes in.
        // SAFETY: inside the 60-byte mapping.
        let bytes = unsafe { std::slice::from_raw_parts(p, 4) };
        assert_eq!(bytes, &[28, 29, 30, 31]);
    }

    #[test]
    fn a_file_shorter_than_the_layer_is_refused() {
        let dir = TempDir::new("shm-short");
        let path = write_layer(&dir, "layer-2.rgba", 4, 4);
        let err = ImageFile::open(&path, 5, 4).unwrap_err();
        assert!(err.contains("needs 80"), "{err}");
        // A longer file is fine: only what the layer needs is mapped.
        ImageFile::open(&path, 2, 2).unwrap();
    }

    #[test]
    fn only_regular_files_are_mapped() {
        let dir = TempDir::new("shm-kind");
        let real = write_layer(&dir, "real.rgba", 2, 2);
        let link = dir.path().join("link.rgba");
        symlink(&real, &link).unwrap();
        assert!(ImageFile::open(&link, 2, 2).is_err());
        assert!(ImageFile::open(dir.path(), 2, 2).is_err());
        assert!(ImageFile::open(&dir.path().join("missing.rgba"), 2, 2).is_err());
    }

    #[test]
    fn a_file_the_app_shrinks_afterwards_is_not_read() {
        let dir = TempDir::new("shm-shrink");
        let path = write_layer(&dir, "layer-3.rgba", 4, 4);
        let image = ImageFile::open(&path, 4, 4).unwrap();
        assert!(image.pixel(0, 0).is_some());
        OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(10)
            .unwrap();
        assert!(image.pixel(0, 0).is_none());
    }

    #[test]
    fn the_mapping_outlives_the_file_being_deleted() {
        let dir = TempDir::new("shm-unlink");
        let path = write_layer(&dir, "layer-4.rgba", 2, 2);
        let image = ImageFile::open(&path, 2, 2).unwrap();
        std::fs::remove_file(&path).unwrap();
        let p = image.pixel(1, 1).unwrap();
        // SAFETY: inside the 16-byte mapping.
        assert_eq!(unsafe { *p }, 12);
    }

    #[test]
    fn a_whole_file_is_read_and_one_over_the_limit_is_refused_unread() {
        let dir = TempDir::new("shm-limited");
        let path = dir.path().join("layer-5.mesh");
        std::fs::write(&path, [7u8; 100]).unwrap();
        assert_eq!(read_limited(&path, 100).unwrap(), vec![7u8; 100]);
        let err = read_limited(&path, 99).unwrap_err();
        assert!(err.contains("100 bytes") && err.contains("99"), "{err}");
        assert_eq!(read_limited(&path, 4096).unwrap().len(), 100);
    }

    #[test]
    fn only_regular_files_are_read_whole() {
        let dir = TempDir::new("shm-limited-kind");
        let real = dir.path().join("real.mesh");
        std::fs::write(&real, [1u8; 8]).unwrap();
        let link = dir.path().join("link.mesh");
        symlink(&real, &link).unwrap();
        assert!(read_limited(&link, 64).is_err());
        assert!(read_limited(dir.path(), 64).is_err());
        assert!(read_limited(&dir.path().join("missing.mesh"), 64).is_err());
    }

    #[test]
    fn a_staged_chunk_is_the_start_of_the_file_and_the_buffer_is_reused_for_the_next() {
        let dir = TempDir::new("shm-staged");
        let path = dir.path().join("atlas-1-0.a8");
        let bytes: Vec<u8> = (0..200u32).map(|i| i as u8).collect();
        std::fs::write(&path, &bytes).unwrap();
        let mut out = Vec::new();
        // What follows the chunk is an older chunk's: left alone.
        read_staged(&path, 120, &mut out).unwrap();
        assert_eq!(out, bytes[..120]);
        let held = out.capacity();
        read_staged(&path, 30, &mut out).unwrap();
        assert_eq!(out, bytes[..30]);
        read_staged(&path, 120, &mut out).unwrap();
        assert_eq!((out.len(), out.capacity()), (120, held), "no second buffer");
        // Nothing at all is a chunk of nothing.
        read_staged(&path, 0, &mut out).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn a_staging_file_shorter_than_its_chunk_is_refused() {
        let dir = TempDir::new("shm-staged-short");
        let path = dir.path().join("atlas-1-1.a8");
        std::fs::write(&path, [9u8; 10]).unwrap();
        let mut out = Vec::new();
        let err = read_staged(&path, 11, &mut out).unwrap_err();
        assert!(
            err.contains("10 bytes") && err.contains("needs 11"),
            "{err}"
        );
        read_staged(&path, 10, &mut out).unwrap();
        assert_eq!(out, [9u8; 10]);
    }

    #[test]
    fn only_regular_files_are_staged_and_nothing_put_in_their_way_blocks() {
        let dir = TempDir::new("shm-staged-kind");
        let real = dir.path().join("real.a8");
        std::fs::write(&real, [1u8; 8]).unwrap();
        let link = dir.path().join("link.a8");
        symlink(&real, &link).unwrap();
        let fifo = dir.path().join("fifo.a8");
        let name = std::ffi::CString::new(fifo.to_str().unwrap()).unwrap();
        // SAFETY: a NUL-terminated path.
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        let mut out = Vec::new();
        // The FIFO has no writer: opening it for reading would wait for one without O_NONBLOCK.
        for in_the_way in [&link, dir.path(), &fifo, &dir.path().join("missing.a8")] {
            assert!(
                read_staged(in_the_way, 4, &mut out).is_err(),
                "{in_the_way:?}"
            );
        }
        read_staged(&real, 4, &mut out).unwrap();
    }
}
