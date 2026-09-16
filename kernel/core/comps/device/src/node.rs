// SPDX-License-Identifier: MPL-2.0

//! The `SysTree` node types the device model owns besides devices: plain
//! directories (roots, index directories, glue directories, driver
//! directories) and symbolic links, and the crate-private view through which
//! the registration sequence edits the tree.

use alloc::{
    collections::BTreeMap,
    string::{String, ToString},
    sync::{Arc, Weak},
};

use aster_systree::{
    BranchNodeFields, SymlinkNodeFields, SysAttrSet, SysBranchNode, SysNode, SysNodeId,
    SysNodeType, SysObj, SysPerms, inherit_sys_symlink_node,
};
use ostd::{
    mm::{VmReader, VmWriter},
    sync::Mutex,
};

use crate::{Error, Result, SysStr};

/// A read-only view of a device-model directory.
#[expect(
    private_bounds,
    reason = "only device-model nodes may implement this trait"
)]
pub trait Container: SysBranchNode + Sealed {}

pub(crate) trait Sealed {}

/// The crate-private editing view of a container.
///
/// Implemented by [`Dir`] and by
/// [`DeviceBase`](crate::DeviceBase), never by a device struct itself, so
/// that a `dyn AnyDevice` or a `dyn Container` cannot reach these operations.
pub(crate) trait SysTreeEdit: Send + Sync {
    /// Adds a child. Fails with [`Error::NameConflict`] if the name is taken.
    fn attach_child(&self, child: Arc<dyn SysObj>) -> Result<()>;

    /// Removes and returns the child with the given name.
    fn detach_child(&self, name: &str) -> Result<Arc<dyn SysObj>>;

    /// Returns whether the container has any children.
    fn has_children(&self) -> bool;

    /// Returns the container's path in the tree.
    fn tree_path(&self) -> String;
}

/// A plain directory in the sysfs tree.
pub(crate) struct Dir {
    fields: BranchNodeFields<dyn SysObj, Self>,
}

impl core::fmt::Debug for Dir {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Dir")
            .field("name", self.fields.name())
            .finish_non_exhaustive()
    }
}

impl Sealed for Dir {}

impl Container for Dir {}

impl Dir {
    /// Creates a directory with no attributes, not yet attached anywhere.
    pub(crate) fn new(name: SysStr) -> Arc<Self> {
        Arc::new_cyclic(|weak_self| Dir {
            fields: BranchNodeFields::new(name, SysAttrSet::new_empty(), weak_self.clone()),
        })
    }
}

impl SysTreeEdit for Dir {
    fn attach_child(&self, child: Arc<dyn SysObj>) -> Result<()> {
        self.fields.add_child(child).map_err(Error::from)
    }

    fn detach_child(&self, name: &str) -> Result<Arc<dyn SysObj>> {
        self.fields.remove_child(name).map_err(Error::from)
    }

    fn has_children(&self) -> bool {
        !self.fields.children_ref().read().is_empty()
    }

    fn tree_path(&self) -> String {
        String::from(SysObj::path(self).as_ref())
    }
}

// `Dir` implements the `SysTree` traits by hand rather than through
// `inherit_sys_branch_node!`, so that `SysBranchNode::remove_child` keeps its
// refusing default: a directory of the device model is edited only through
// [`SysTreeEdit`].
impl SysObj for Dir {
    fn as_any(&self) -> &dyn core::any::Any {
        self
    }

    fn cast_to_node(&self) -> Option<Arc<dyn SysNode>> {
        self.fields
            .weak_self()
            .upgrade()
            .map(|dir| dir as Arc<dyn SysNode>)
    }

    fn cast_to_branch(&self) -> Option<Arc<dyn SysBranchNode>> {
        self.fields
            .weak_self()
            .upgrade()
            .map(|dir| dir as Arc<dyn SysBranchNode>)
    }

    fn id(&self) -> &SysNodeId {
        self.fields.id()
    }

    fn type_(&self) -> SysNodeType {
        SysNodeType::Branch
    }

    fn name(&self) -> &SysStr {
        self.fields.name()
    }

    fn init_parent(&self, parent: Weak<dyn SysBranchNode>) {
        self.fields.init_parent(parent);
    }

    fn parent(&self) -> Option<Arc<dyn SysBranchNode>> {
        self.fields.parent()
    }
}

impl SysNode for Dir {
    fn node_attrs(&self) -> Arc<SysAttrSet> {
        self.fields.attr_set().clone()
    }

    fn is_attr_absent(&self, _name: &str) -> bool {
        false
    }

    fn read_attr(&self, _name: &str, _writer: &mut VmWriter) -> aster_systree::Result<usize> {
        Err(aster_systree::Error::NotFound)
    }

    fn write_attr(&self, _name: &str, _reader: &mut VmReader) -> aster_systree::Result<usize> {
        Err(aster_systree::Error::NotFound)
    }

    fn read_attr_at(
        &self,
        name: &str,
        _offset: usize,
        writer: &mut VmWriter,
    ) -> aster_systree::Result<usize> {
        self.read_attr(name, writer)
    }

    fn write_attr_at(
        &self,
        name: &str,
        _offset: usize,
        reader: &mut VmReader,
    ) -> aster_systree::Result<usize> {
        self.write_attr(name, reader)
    }

    fn perms(&self) -> SysPerms {
        SysPerms::DEFAULT_RW_PERMS
    }
}

impl SysBranchNode for Dir {
    fn visit_child_with(&self, name: &str, f: &mut dyn FnMut(Option<&Arc<dyn SysObj>>)) {
        self.fields.visit_child_with(name, f);
    }

    fn visit_children_with(
        &self,
        min_id: u64,
        f: &mut dyn for<'a> FnMut(&'a Arc<dyn SysObj>) -> Option<()>,
    ) {
        self.fields.visit_children_with(min_id, f);
    }

    fn child(&self, name: &str) -> Option<Arc<dyn SysObj>> {
        self.fields.child(name)
    }
}

/// A symbolic link in the sysfs tree.
#[derive(Debug)]
pub(crate) struct SymlinkNode {
    fields: SymlinkNodeFields<Self>,
}

impl SymlinkNode {
    /// Creates a symlink with a literal target.
    pub(crate) fn new(name: SysStr, target: String) -> Arc<Self> {
        Arc::new_cyclic(|weak_self| {
            let fields = SymlinkNodeFields::new(name, target, weak_self.clone());
            SymlinkNode { fields }
        })
    }
}

inherit_sys_symlink_node!(SymlinkNode, fields);

/// Adds to `dir` a symlink named `name` whose target is the node at
/// `target_path`, expressed relative to `dir` as Linux does.
pub(crate) fn add_link(dir: &dyn SysTreeEdit, name: &str, target_path: &str) -> Result<()> {
    let target = relative_path(&dir.tree_path(), target_path);
    let link = SymlinkNode::new(SysStr::from(name.to_string()), target);
    dir.attach_child(link)
}

/// Removes the symlink named `name` from `dir`, ignoring its absence.
pub(crate) fn remove_link(dir: &dyn SysTreeEdit, name: &str) {
    let _ = dir.detach_child(name);
}

/// Glue directories owned by one container, one per class of child.
///
/// A glue directory is created when the first device of a class is placed
/// under the container and dropped when the last one leaves. Both
/// transitions happen under one lock, so a device cannot be attached into a
/// glue directory that is being dropped (Linux's `gdp_mutex` serves the same
/// purpose).
pub(crate) struct GlueDirs {
    dirs: Mutex<BTreeMap<SysStr, Arc<Dir>>>,
}

impl GlueDirs {
    pub(crate) const fn new() -> Self {
        Self {
            dirs: Mutex::new(BTreeMap::new()),
        }
    }

    /// Attaches `child` into the glue directory `name` under `owner`,
    /// creating the directory if needed, and returns that directory.
    pub(crate) fn attach_into(
        &self,
        name: &str,
        owner: &dyn SysTreeEdit,
        child: Arc<dyn SysObj>,
    ) -> Result<Arc<Dir>> {
        let mut dirs = self.dirs.lock();
        if let Some(dir) = dirs.get(name) {
            dir.attach_child(child)?;
            return Ok(dir.clone());
        }
        let dir = Dir::new(SysStr::from(name.to_string()));
        owner.attach_child(dir.clone())?;
        if let Err(e) = dir.attach_child(child) {
            let _ = owner.detach_child(name);
            return Err(e);
        }
        dirs.insert(SysStr::from(name.to_string()), dir.clone());
        Ok(dir)
    }

    /// Drops the glue directory `name` under `owner` if it is now empty.
    pub(crate) fn drop_if_empty(&self, name: &str, owner: &dyn SysTreeEdit) {
        let mut dirs = self.dirs.lock();
        if let Some(dir) = dirs.get(name)
            && !dir.has_children()
        {
            let _ = owner.detach_child(name);
            dirs.remove(name);
        }
    }
}

fn relative_path(from_dir: &str, to: &str) -> String {
    let from: alloc::vec::Vec<&str> = from_dir.split('/').filter(|s| !s.is_empty()).collect();
    let to: alloc::vec::Vec<&str> = to.split('/').filter(|s| !s.is_empty()).collect();
    let Some((last, to_parent)) = to.split_last() else {
        // The target is the root, which no symlink should point to.
        return String::from("/");
    };
    let common = from
        .iter()
        .zip(to_parent.iter())
        .take_while(|(a, b)| a == b)
        .count();

    let mut result = String::new();
    for _ in common..from.len() {
        result.push_str("../");
    }
    for component in &to_parent[common..] {
        result.push_str(component);
        result.push('/');
    }
    result.push_str(last);
    result
}
