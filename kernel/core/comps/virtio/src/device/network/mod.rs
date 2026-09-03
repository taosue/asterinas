// SPDX-License-Identifier: MPL-2.0

mod buffer;
mod config;
pub mod device;
mod header;

pub(crate) fn init() {
    buffer::init();
}
