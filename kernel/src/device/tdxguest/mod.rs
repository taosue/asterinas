// SPDX-License-Identifier: MPL-2.0

use core::{mem::size_of, time::Duration};

use align_ext::AlignExt;
use aster_util::{field_ptr, safe_ptr::SafePtr};
use device_id::DeviceId;
use ostd::{
    mm::{
        io_util::HasVmReaderWriter, DmaCoherent, FrameAllocOptions, HasPaddr, HasSize, USegment,
        VmIo, PAGE_SIZE,
    },
    sync::WaitQueue,
};
use tdx_guest::{
    tdcall::{get_report, TdCallError},
    tdvmcall::{get_quote, TdVmcallError},
    SHARED_MASK,
};

use crate::{
    events::IoEvents,
    fs::{
        device::{Device, DeviceType},
        inode_handle::FileIo,
        utils::{IoctlCmd, StatusFlags},
    },
    prelude::*,
    process::signal::{PollHandle, Pollable},
};

const TDX_REPORTDATA_LEN: usize = 64;
const TDX_REPORT_LEN: usize = 1024;

#[derive(Debug, Clone, Copy, Pod)]
#[repr(C)]
pub struct TdxReportRequest {
    report_data: [u8; TDX_REPORTDATA_LEN],
    tdx_report: [u8; TDX_REPORT_LEN],
}

pub struct TdxGuest;

impl Device for TdxGuest {
    fn type_(&self) -> DeviceType {
        DeviceType::Misc
    }

    fn id(&self) -> DeviceId {
        DeviceId::new(0xa, 0x7b)
    }
}

impl From<TdCallError> for Error {
    fn from(err: TdCallError) -> Self {
        match err {
            TdCallError::TdxNoValidVeInfo => {
                Error::with_message(Errno::EINVAL, "TdCallError::TdxNoValidVeInfo")
            }
            TdCallError::TdxOperandInvalid => {
                Error::with_message(Errno::EINVAL, "TdCallError::TdxOperandInvalid")
            }
            TdCallError::TdxPageAlreadyAccepted => {
                Error::with_message(Errno::EINVAL, "TdCallError::TdxPageAlreadyAccepted")
            }
            TdCallError::TdxPageSizeMismatch => {
                Error::with_message(Errno::EINVAL, "TdCallError::TdxPageSizeMismatch")
            }
            TdCallError::TdxOperandBusy => {
                Error::with_message(Errno::EBUSY, "TdCallError::TdxOperandBusy")
            }
            TdCallError::Other => Error::with_message(Errno::EAGAIN, "TdCallError::Other"),
            _ => todo!(),
        }
    }
}

impl From<TdVmcallError> for Error {
    fn from(err: TdVmcallError) -> Self {
        match err {
            TdVmcallError::TdxRetry => {
                Error::with_message(Errno::EINVAL, "TdVmcallError::TdxRetry")
            }
            TdVmcallError::TdxOperandInvalid => {
                Error::with_message(Errno::EINVAL, "TdVmcallError::TdxOperandInvalid")
            }
            TdVmcallError::TdxGpaInuse => {
                Error::with_message(Errno::EINVAL, "TdVmcallError::TdxGpaInuse")
            }
            TdVmcallError::TdxAlignError => {
                Error::with_message(Errno::EINVAL, "TdVmcallError::TdxAlignError")
            }
            TdVmcallError::Other => Error::with_message(Errno::EAGAIN, "TdVmcallError::Other"),
        }
    }
}

impl Pollable for TdxGuest {
    fn poll(&self, mask: IoEvents, _poller: Option<&mut PollHandle>) -> IoEvents {
        let events = IoEvents::IN | IoEvents::OUT;
        events & mask
    }
}

impl FileIo for TdxGuest {
    fn read(&self, _writer: &mut VmWriter, _status_flags: StatusFlags) -> Result<usize> {
        return_errno_with_message!(Errno::EPERM, "Read operation not supported")
    }

    fn write(&self, _reader: &mut VmReader, _status_flags: StatusFlags) -> Result<usize> {
        return_errno_with_message!(Errno::EPERM, "Write operation not supported")
    }

    fn ioctl(&self, cmd: IoctlCmd, arg: usize) -> Result<i32> {
        match cmd {
            IoctlCmd::TDXGETREPORT => handle_get_report(arg),
            _ => return_errno_with_message!(Errno::EPERM, "Unsupported ioctl"),
        }
    }
}

pub fn tdx_get_quote(inblob: &[u8]) -> Result<Box<[u8]>> {
    const GET_QUOTE_IN_FLIGHT: u64 = 0xFFFF_FFFF_FFFF_FFFF;
    const GET_QUOTE_BUF_SIZE: usize = 8 * 1024;

    let report = tdx_get_report(inblob)?;
    let buf = alloc_dma_buf(GET_QUOTE_BUF_SIZE)?;
    let report_ptr: SafePtr<TdxQuoteHdr, _, _> = SafePtr::new(&buf, 0);

    field_ptr!(&report_ptr, TdxQuoteHdr, version).write(&1u64)?;
    field_ptr!(&report_ptr, TdxQuoteHdr, status).write(&0u64)?;
    field_ptr!(&report_ptr, TdxQuoteHdr, in_len).write(&(TDX_REPORT_LEN as u32))?;
    field_ptr!(&report_ptr, TdxQuoteHdr, out_len).write(&0u32)?;
    buf.write(
        size_of::<TdxQuoteHdr>(),
        report.reader().to_fallible().limit(TDX_REPORT_LEN),
    )?;

    // FIXME: The `get_quote` API from the `tdx_guest` crate should have been marked `unsafe`
    // because it has no way to determine if the input physical address is safe or not.
    get_quote((buf.paddr() as u64) | SHARED_MASK, buf.size() as u64)?;

    // Poll for the quote to be ready.
    let status_ptr = field_ptr!(&report_ptr, TdxQuoteHdr, status);
    let sleep_queue = WaitQueue::new();
    let sleep_duration = Duration::from_millis(100);
    loop {
        let status = status_ptr.read()?;
        if status != GET_QUOTE_IN_FLIGHT {
            break;
        }
        let _ = sleep_queue.wait_until_or_timeout(|| -> Option<()> { None }, &sleep_duration);
    }

    // Note: We cannot convert `DmaCoherent` to `USegment` here. When shared memory is converted back
    // to private memory in TDX, `TDG.MEM.PAGE.ACCEPT` will zero out all content.
    // TDX Module Specification - `TDG.MEM.PAGE.ACCEPT` Leaf:
    // "Accept a pending private page and initialize it to all-0 using the TD ephemeral private key."
    let out_len = field_ptr!(&report_ptr, TdxQuoteHdr, out_len).read()?;
    let mut outblob = vec![0u8; out_len as usize].into_boxed_slice();
    buf.read_bytes(size_of::<TdxQuoteHdr>(), outblob.as_mut())?;
    Ok(outblob)
}

#[repr(C)]
struct TdxQuoteHdr {
    // Quote version, filled by TD
    version: u64,
    // Status code of quote request, filled by VMM
    status: u64,
    // Length of TDREPORT, filled by TD
    in_len: u32,
    // Length of quote, filled by VMM
    out_len: u32,
}

fn handle_get_report(arg: usize) -> Result<i32> {
    let current_task = ostd::task::Task::current().unwrap();
    let user_space = CurrentUserSpace::new(current_task.as_thread_local().unwrap());
    let user_request: TdxReportRequest = user_space.read_val(arg)?;

    let report = tdx_get_report(&user_request.report_data)?;

    let tdx_report_vaddr = arg + TDX_REPORTDATA_LEN;
    user_space.write_bytes(tdx_report_vaddr, report.reader().limit(TDX_REPORT_LEN))?;
    Ok(0)
}

/// Gets the TDX report given the specified data in `inblob`.
///
/// The first `TDX_REPORT_LEN` bytes of data in the returned `USegment` is the report.
/// The rest in `USegment` should be ignored.
fn tdx_get_report(inblob: &[u8]) -> Result<USegment> {
    if inblob.len() != TDX_REPORTDATA_LEN {
        return_errno_with_message!(Errno::EINVAL, "Invalid inblob length");
    }

    let report: USegment = FrameAllocOptions::new().alloc_segment(1)?.into();

    // Use `inblob` as the data associated with the report.
    let report_data_paddr = {
        // The `get_report` function requires report data to be 64-byte aligned.
        //
        // From TDX Module Specification, the report structure returned by TDX Module
        // places the report data at offset 128, so using the same offset keeps the
        // memory layout consistent with the TDX Modules's output format. And we can
        // directly call `get_report` on the existing report structure without needing
        // to rewrite the report data.
        const REPORT_DATA_OFFSET: usize = 128;
        report.write_bytes(REPORT_DATA_OFFSET, inblob).unwrap();
        report.paddr() + REPORT_DATA_OFFSET
    };

    // FIXME: The `get_report` API from the `tdx_guest` crate should have been marked `unsafe`
    // because it has no way to determine if the input physical address is safe or not.
    get_report(report.paddr() as u64, report_data_paddr as u64)?;
    Ok(report)
}

fn alloc_dma_buf(buf_len: usize) -> Result<DmaCoherent> {
    let aligned_buf_len = buf_len.align_up(PAGE_SIZE);
    let segment = FrameAllocOptions::new().alloc_segment(aligned_buf_len / PAGE_SIZE)?;

    let dma_buf = DmaCoherent::map(segment.into(), false).unwrap();
    Ok(dma_buf)
}
