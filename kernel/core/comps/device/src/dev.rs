// SPDX-License-Identifier: MPL-2.0

//! The `/sys/dev` device-number index.

use alloc::{format, sync::Arc};

use aster_systree::{
    BranchNodeFields, SymlinkNodeFields, SysAttrSet, SysObj, SysPerms, SysStr,
    inherit_sys_branch_node, inherit_sys_symlink_node,
};
use device_id::DeviceId;
use spin::Once;

/// The type of a device node indexed under `/sys/dev`.
#[derive(Debug)]
pub enum DeviceType {
    Char,
    Block,
}

static DEV_ROOT: Once<Arc<DevSysNodeRoot>> = Once::new();

/// Adds the `/sys/dev/{char,block}/<major>:<minor>` link for `path`.
pub fn register_dev_node(
    path: &str,
    device_type: DeviceType,
    id: DeviceId,
) -> aster_systree::Result<()> {
    let root = DEV_ROOT.get().unwrap();
    let dev_type = match device_type {
        DeviceType::Char => &root.char_dir,
        DeviceType::Block => &root.block_dir,
    };
    dev_type.fields.add_child(DevNodeLink::new(path, id))
}

/// Initializes `/sys/dev`, `/sys/dev/char`, and `/sys/dev/block`.
pub(super) fn init() {
    DEV_ROOT.call_once(|| {
        let root = DevSysNodeRoot::new();
        aster_systree::primary_tree()
            .root()
            .add_child(root.clone())
            .unwrap();
        root
    });
}

#[derive(Debug)]
struct DevSysNodeRoot {
    fields: BranchNodeFields<dyn SysObj, Self>,
    char_dir: Arc<DevTypeSysNodeRoot>,
    block_dir: Arc<DevTypeSysNodeRoot>,
}

impl DevSysNodeRoot {
    fn new() -> Arc<Self> {
        Arc::new_cyclic(|weak_self| {
            let char_dir = DevTypeSysNodeRoot::new("char");
            let block_dir = DevTypeSysNodeRoot::new("block");
            let fields = BranchNodeFields::new(
                SysStr::from("dev"),
                SysAttrSet::new_empty(),
                weak_self.clone(),
            );
            fields
                .add_child(char_dir.clone() as Arc<dyn SysObj>)
                .unwrap();
            fields
                .add_child(block_dir.clone() as Arc<dyn SysObj>)
                .unwrap();
            Self {
                fields,
                char_dir,
                block_dir,
            }
        })
    }
}

inherit_sys_branch_node!(DevSysNodeRoot, fields, {
    fn perms(&self) -> SysPerms {
        SysPerms::DEFAULT_RO_PERMS
    }
});

#[derive(Debug)]
struct DevTypeSysNodeRoot {
    fields: BranchNodeFields<dyn SysObj, Self>,
}

impl DevTypeSysNodeRoot {
    fn new(name: &'static str) -> Arc<Self> {
        Arc::new_cyclic(|weak_self| Self {
            fields: BranchNodeFields::new(
                SysStr::from(name),
                SysAttrSet::new_empty(),
                weak_self.clone(),
            ),
        })
    }
}

inherit_sys_branch_node!(DevTypeSysNodeRoot, fields, {
    fn perms(&self) -> SysPerms {
        SysPerms::DEFAULT_RO_PERMS
    }
});

#[derive(Debug)]
struct DevNodeLink {
    fields: SymlinkNodeFields<Self>,
}

impl DevNodeLink {
    fn new(path: &str, id: DeviceId) -> Arc<Self> {
        Arc::new_cyclic(|weak_self| Self {
            fields: SymlinkNodeFields::new(
                SysStr::from(format!("{}:{}", id.major().get(), id.minor().get())),
                format!("../..{}", path),
                weak_self.clone(),
            ),
        })
    }
}

inherit_sys_symlink_node!(DevNodeLink, fields, {});
