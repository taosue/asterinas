// SPDX-License-Identifier: MPL-2.0

//! The device model's interface to the kernel's devtmpfs implementation.

use alloc::sync::Arc;

use spin::Once;

use crate::{DevNodeRequest, Error, Result};

/// A device-node operation failed.
#[derive(Clone, Copy, Debug)]
pub struct HookError;

/// Creates and deletes device nodes on behalf of the device model.
///
/// Hooks run under the device lifecycle mutex and must not call back into
/// registration or removal. A failed creation must leave no device node behind.
pub trait KernelHooks: Send + Sync + 'static {
    /// Creates a node, including any intermediate directories in its path.
    fn create_devnode(&self, request: &DevNodeRequest) -> Result<(), HookError>;

    /// Deletes a node using the request that originally created it.
    ///
    /// On failure, the device stays registered and removal can be retried.
    fn delete_devnode(&self, request: &DevNodeRequest) -> Result<(), HookError>;
}

static HOOKS: Once<Arc<dyn KernelHooks>> = Once::new();

/// Installs device-node hooks before registering any numbered devices.
///
/// Subsequent calls have no effect. Registration of a numbered device fails
/// if hooks have not been installed, so success always means its node exists.
pub fn install_hooks(hooks: Arc<dyn KernelHooks>) {
    HOOKS.call_once(|| hooks);
}

pub(crate) fn create_devnode(request: &DevNodeRequest) -> Result<()> {
    HOOKS
        .get()
        .ok_or(Error::Hook)?
        .create_devnode(request)
        .map_err(|_| Error::Hook)
}

pub(crate) fn delete_devnode(request: &DevNodeRequest) -> Result<()> {
    HOOKS
        .get()
        .ok_or(Error::Hook)?
        .delete_devnode(request)
        .map_err(|_| Error::Hook)
}
