// SPDX-License-Identifier: MPL-2.0

use ostd::mm::{DmaCoherent, FrameAllocOptions, HasPaddr, VmIo, PAGE_SIZE};
use alloc::vec;

use tdx_guest::{
    tdcall::{get_report, TdCallError},
    tdvmcall::{get_quote, TdVmcallError},
    SHARED_MASK,
};

use super::*;
use crate::{
    error::Error,
    events::IoEvents,
    fs::{inode_handle::FileIo, utils::IoctlCmd},
    process::signal::{PollHandle, Pollable},
};

const TDX_REPORTDATA_LEN: usize = 64;
const TDX_REPORT_LEN: usize = 1024;

#[derive(Debug, Clone, Copy, Pod)]
#[repr(C)]
pub struct TdxQuoteRequest {
    buf: usize,
    len: usize,
}

#[repr(align(64))]
#[repr(C)]
struct ReportDataWapper {
    report_data: [u8; TDX_REPORTDATA_LEN],
}

#[repr(align(1024))]
#[repr(C)]
struct TdxReportWapper {
    tdx_report: [u8; TDX_REPORT_LEN],
}

struct QuoteEntry {
    // Kernel buffer to share data with VMM (size is page aligned)
    buf: DmaCoherent,
    // Size of the allocated memory
    buf_len: usize,
}

#[repr(C)]
struct tdx_quote_hdr {
    // Quote version, filled by TD
    version: u64,
    // Status code of Quote request, filled by VMM
    status: u64,
    // Length of TDREPORT, filled by TD
    in_len: u32,
    // Length of Quote, filled by VMM
    out_len: u32,
    // Actual Quote data or TDREPORT on input
    data: Vec<u64>,
}

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
    fn read(&self, _writer: &mut VmWriter) -> Result<usize> {
        return_errno_with_message!(Errno::EPERM, "Read operation not supported")
    }

    fn write(&self, _reader: &mut VmReader) -> Result<usize> {
        return_errno_with_message!(Errno::EPERM, "Write operation not supported")
    }

    fn ioctl(&self, cmd: IoctlCmd, arg: usize) -> Result<i32> {
        match cmd {
            IoctlCmd::TDXGETREPORT => handle_get_report(arg),
            IoctlCmd::TDXGETQUOTE => handle_get_quote(arg),
            _ => return_errno_with_message!(Errno::EPERM, "Unsupported ioctl"),
        }
    }
}

fn handle_get_report(arg: usize) -> Result<i32> {
    const SHARED_BIT: u8 = 51;
    const SHARED_MASK: u64 = 1u64 << SHARED_BIT;
    let current_task = ostd::task::Task::current().unwrap();
    let user_space = CurrentUserSpace::new(current_task.as_thread_local().unwrap());
    let user_request: TdxReportRequest = user_space.read_val(arg)?;

    let segment = FrameAllocOptions::new().alloc_segment(2).unwrap();
    let dma_coherent = DmaCoherent::map(segment.into(), false).unwrap();
    dma_coherent
        .write_bytes(0, &user_request.report_data)
        .unwrap();
    // 1024-byte alignment.
    dma_coherent
        .write_bytes(1024, &user_request.tdx_report)
        .unwrap();

    if let Err(err) = get_report(
        ((dma_coherent.paddr() + 1024) as u64) | SHARED_MASK,
        (dma_coherent.paddr() as u64) | SHARED_MASK,
    ) {
        println!("[kernel]: get TDX report error: {:?}", err);
        return Err(err.into());
    }

    let tdx_report_vaddr = arg + TDX_REPORTDATA_LEN;
    let mut generated_report = vec![0u8; TDX_REPORT_LEN];
    dma_coherent
        .read_bytes(1024, &mut generated_report)
        .unwrap();
    let report_slice: &[u8] = &generated_report;
    user_space.write_bytes(tdx_report_vaddr, &mut VmReader::from(report_slice))?;
    Ok(0)
}

fn handle_get_quote(arg: usize) -> Result<i32> {
    const GET_QUOTE_IN_FLIGHT: u64 = 0xFFFF_FFFF_FFFF_FFFF;
    let current_task = ostd::task::Task::current().unwrap();
    let user_space = CurrentUserSpace::new(current_task.as_thread_local().unwrap());
    let tdx_quote: TdxQuoteRequest = user_space.read_val(arg)?;
    if tdx_quote.len == 0 {
        return Err(Error::with_message(Errno::EBUSY, "Invalid parameter"));
    }
    let entry = alloc_quote_entry(tdx_quote.len);

    // Copy data (with TDREPORT) from user buffer to kernel Quote buffer
    let mut quote_buffer = vec![0u8; entry.buf_len];
    user_space.read_bytes(tdx_quote.buf, &mut (&mut quote_buffer[..]).into())?;
    entry.buf.write_bytes(0, &quote_buffer)?;

    if let Err(err) = get_quote(
        (entry.buf.paddr() as u64) | SHARED_MASK,
        entry.buf_len as u64,
    ) {
        println!("[kernel] get quote error: {:?}", err);
        return Err(err.into());
    }

    // Poll for the quote to be ready.
    loop {
        entry.buf.read_bytes(0, &mut quote_buffer)?;
        let quote_hdr: tdx_quote_hdr = parse_quote_header(&quote_buffer);
        if quote_hdr.status != GET_QUOTE_IN_FLIGHT {
            break;
        }
    }
    entry.buf.read_bytes(0, &mut quote_buffer)?;

    let quote_slice: &[u8] = &quote_buffer;
    user_space.write_bytes(tdx_quote.buf, &mut VmReader::from(quote_slice))?;
    Ok(0)
}

fn alloc_quote_entry(buf_len: usize) -> QuoteEntry {
    const PAGE_MASK: usize = PAGE_SIZE - 1;
    let aligned_buf_len = buf_len & (!PAGE_MASK);

    let segment = FrameAllocOptions::new().alloc_segment(aligned_buf_len / PAGE_SIZE).unwrap();
    let dma_buf = DmaCoherent::map(segment.into(), false).unwrap();

    let entry = QuoteEntry {
        buf: dma_buf,
        buf_len: aligned_buf_len as usize,
    };
    entry
}

fn parse_quote_header(buffer: &[u8]) -> tdx_quote_hdr {
    let version = u64::from_be_bytes(buffer[0..8].try_into().unwrap());
    let status = u64::from_be_bytes(buffer[8..16].try_into().unwrap());
    let in_len = u32::from_be_bytes(buffer[16..20].try_into().unwrap());
    let out_len = u32::from_be_bytes(buffer[20..24].try_into().unwrap());

    tdx_quote_hdr {
        version,
        status,
        in_len,
        out_len,
        data: vec![0],
    }
}