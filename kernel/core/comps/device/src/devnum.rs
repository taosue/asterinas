// SPDX-License-Identifier: MPL-2.0

//! Device numbers and `/dev` node requests.

use core::fmt;

use device_id::DeviceId;

use crate::SysStr;

/// Whether a device number names a character or a block device.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DevKind {
    Char,
    Block,
}

/// A device number together with its kind.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DevNum {
    kind: DevKind,
    id: DeviceId,
}

impl DevNum {
    /// Creates a character device number.
    pub fn char(id: DeviceId) -> Self {
        Self {
            kind: DevKind::Char,
            id,
        }
    }

    /// Creates a block device number.
    pub fn block(id: DeviceId) -> Self {
        Self {
            kind: DevKind::Block,
            id,
        }
    }

    /// Returns the kind.
    pub fn kind(&self) -> DevKind {
        self.kind
    }

    /// Returns the major and minor number.
    pub fn id(&self) -> DeviceId {
        self.id
    }
}

impl fmt::Display for DevNum {
    /// Formats as `major:minor`, the form used by `/sys/dev` and the `dev`
    /// attribute.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.id.major().get(), self.id.minor().get())
    }
}

/// A request to create or delete a `/dev` node.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DevNodeRequest {
    /// The device number the node refers to.
    devnum: DevNum,
    /// The path of the node relative to `/dev`, e.g. `null` or `input/event0`.
    path: SysStr,
    /// The permission bits of the node.
    mode: u16,
}

/// The default mode of a device node when the class does not override it (Linux devtmpfs uses `0600` as well).
pub(crate) const DEFAULT_DEVNODE_MODE: u16 = 0o600;

impl DevNodeRequest {
    pub(crate) fn new(devnum: DevNum, path: SysStr, mode: u16) -> crate::Result<Self> {
        if mode & !0o7777 != 0
            || path.contains('\0')
            || path.split('/').any(|part| matches!(part, "" | "." | ".."))
        {
            return Err(crate::Error::InvalidValue);
        }
        Ok(Self { devnum, path, mode })
    }

    /// Returns the device number and kind.
    pub fn devnum(&self) -> DevNum {
        self.devnum
    }

    /// Returns the path relative to `/dev`.
    pub fn path(&self) -> &SysStr {
        &self.path
    }

    /// Returns the initial permission bits.
    pub fn mode(&self) -> u16 {
        self.mode
    }
}

/// Overrides the path or initial permissions of a class device's `/dev` node.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DevNode {
    /// The path relative to `/dev`; `None` uses the device name with `!` replaced by `/`.
    pub path: Option<SysStr>,
    /// The initial permission bits; `None` uses `0600`.
    pub mode: Option<u16>,
}
