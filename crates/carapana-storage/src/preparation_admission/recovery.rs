//! A private OS-lock witness for never-exported preparations, not process/consumer discovery.

use std::{fs::File, io, path::Path};

pub(super) struct OwnerLock {
    directory: File,
    file: File,
    name: String,
    identity: [u8; 16],
}

impl OwnerLock {
    pub(super) fn identity(&self) -> &[u8; 16] {
        &self.identity
    }

    pub(super) fn create(root: &Path, id: i64) -> io::Result<Self> {
        let directory = open_directory(root)?;
        // Interrupted registrations may leave empty controls without a committed owner record.
        // Never allow repeated crashes/recovery to grow control metadata without a finite bound.
        ensure_room(&directory, 4096)?;
        let name = control_name(id)?;
        let file = open_control(&directory, &name, true)?;
        let identity = validate(&directory, &file)?;
        if !try_lock(&file)? {
            return Err(io::Error::other(
                "new preparation lock is unexpectedly busy",
            ));
        }
        file.sync_all()?;
        directory.sync_all()?;
        Ok(Self {
            directory,
            file,
            name,
            identity,
        })
    }

    pub(super) fn try_recover(
        root: &Path,
        id: i64,
        expected: [u8; 16],
    ) -> io::Result<Option<Self>> {
        let directory = open_directory(root)?;
        let name = control_name(id)?;
        let file = open_control(&directory, &name, false)?;
        let identity = validate(&directory, &file)?;
        if identity != expected {
            return Err(io::Error::other("preparation lock identity changed"));
        }
        if !try_lock(&file)? {
            return Ok(None);
        }
        Ok(Some(Self {
            directory,
            file,
            name,
            identity,
        }))
    }

    pub(super) fn remove(&self) -> io::Result<()> {
        let current = open_control(&self.directory, &self.name, false)?;
        if validate(&self.directory, &current)? != self.identity
            || validate(&self.directory, &self.file)? != self.identity
        {
            return Err(io::Error::other("preparation lock identity changed"));
        }
        unlink(&self.directory, &self.name)?;
        self.directory.sync_all()
    }
}

fn control_name(id: i64) -> io::Result<String> {
    if id <= 0 {
        return Err(io::Error::other("invalid preparation identity"));
    }
    Ok(format!("{id}.lock"))
}

#[cfg(unix)]
pub(super) fn ensure_room(directory: &File, limit: usize) -> io::Result<()> {
    use rustix::fs::{Dir, Mode, OFlags, openat};
    let descriptor = openat(
        directory,
        ".",
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )?;
    let stream = Dir::new(descriptor)?;
    for (index, entry) in stream.enumerate() {
        entry?;
        if index >= limit {
            return Err(io::Error::other(
                "preparation control entry budget exceeded",
            ));
        }
    }
    Ok(())
}

#[cfg(not(unix))]
pub(super) fn ensure_room(_: &File, _: usize) -> io::Result<()> {
    Err(io::ErrorKind::Unsupported.into())
}

#[cfg(unix)]
fn open_directory(root: &Path) -> io::Result<File> {
    use rustix::fs::{Mode, OFlags, open};
    use std::os::unix::fs::MetadataExt;
    crate::user_database::ensure_private_directory(root)
        .map_err(|_| io::Error::other("unsafe preparation lock directory"))?;
    let directory = File::from(open(
        root,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )?);
    if directory.metadata()?.mode() & 0o077 != 0 {
        return Err(io::Error::other("unsafe preparation lock directory"));
    }
    Ok(directory)
}

#[cfg(unix)]
fn open_control(directory: &File, name: &str, create: bool) -> io::Result<File> {
    use rustix::fs::{Mode, OFlags, openat};
    let mut flags = OFlags::RDWR | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK;
    if create {
        flags |= OFlags::CREATE | OFlags::EXCL;
    }
    Ok(File::from(openat(
        directory,
        name,
        flags,
        Mode::from_raw_mode(0o600),
    )?))
}

#[cfg(unix)]
fn validate(directory: &File, file: &File) -> io::Result<[u8; 16]> {
    use std::os::unix::fs::MetadataExt;
    let metadata = file.metadata()?;
    if !metadata.is_file()
        || metadata.nlink() != 1
        || metadata.len() != 0
        || metadata.mode() & 0o7777 != 0o600
        || metadata.uid() != directory.metadata()?.uid()
    {
        return Err(io::Error::other("unsafe preparation lock"));
    }
    let mut identity = [0_u8; 16];
    identity[..8].copy_from_slice(&metadata.dev().to_be_bytes());
    identity[8..].copy_from_slice(&metadata.ino().to_be_bytes());
    Ok(identity)
}

#[cfg(unix)]
fn try_lock(file: &File) -> io::Result<bool> {
    use rustix::fs::{FlockOperation, flock};
    match flock(file, FlockOperation::NonBlockingLockExclusive) {
        Ok(()) => Ok(true),
        Err(rustix::io::Errno::WOULDBLOCK) => Ok(false),
        Err(error) => Err(error.into()),
    }
}

#[cfg(unix)]
fn unlink(directory: &File, name: &str) -> io::Result<()> {
    rustix::fs::unlinkat(directory, name, rustix::fs::AtFlags::empty()).map_err(Into::into)
}

#[cfg(not(unix))]
fn open_directory(_: &Path) -> io::Result<File> {
    Err(io::ErrorKind::Unsupported.into())
}
#[cfg(not(unix))]
fn open_control(_: &File, _: &str, _: bool) -> io::Result<File> {
    Err(io::ErrorKind::Unsupported.into())
}
#[cfg(not(unix))]
fn validate(_: &File, _: &File) -> io::Result<[u8; 16]> {
    Err(io::ErrorKind::Unsupported.into())
}
#[cfg(not(unix))]
fn try_lock(_: &File) -> io::Result<bool> {
    Err(io::ErrorKind::Unsupported.into())
}
#[cfg(not(unix))]
fn unlink(_: &File, _: &str) -> io::Result<()> {
    Err(io::ErrorKind::Unsupported.into())
}
