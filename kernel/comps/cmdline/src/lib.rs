// SPDX-License-Identifier: MPL-2.0

//! The module to parse kernel command-line arguments.
//!
//! The format of the Asterinas command line string conforms
//! to the Linux kernel command line rules:
//!
//! <https://www.kernel.org/doc/html/v6.4/admin-guide/kernel-parameters.html>
//!
#![no_std]
#![deny(unsafe_code)]

extern crate alloc;

use alloc::{ffi::CString, string::ToString, vec::Vec};

use component::{ComponentInitError, init_component};
use ostd::boot::cmdline::{KernelParam, query_kernel_param};
use spin::Once;

/// Declares a kernel command-line parameter for the *cmdline component*.
///
/// This is a convenience wrapper around [`ostd::define_kernel_param!`]. It always
/// registers the parameter as a non-early parameter (`early = false`).
///
/// Prefer using this wrapper instead of calling `ostd::define_kernel_param!` directly:
/// the cmdline component is responsible for parsing the boot command line and
/// invoking the registered setup callbacks, so using this macro makes the intended
/// dependency explicit and helps avoid initialization-order issues.
///
/// # Syntax
/// - `name`: Parameter name string literal.
///   - Use a trailing `=` for parameters that take a value, e.g. `"foo="` for `foo=123`.
///   - Omit `=` for flag-style parameters without a value, e.g. `"bar"` for `bar`.
/// - `slot`: A `'static` storage location of type `spin::Once<T>`, where `T: FromKernelParam`.
///
/// # Behavior
/// When the cmdline parser encounters `name`, it will parse the optional value via
/// `T: FromKernelParam` and store the first successfully parsed value into `slot`.
/// Subsequent occurrences of the same parameter are ignored because `spin::Once<T>`
/// can only be initialized once.
///
/// # Examples
/// ```no_run
/// static LOG_LEVEL: spin::Once<LogLevel> = spin::Once::new();
/// kernel_param!("log_level=", LOG_LEVEL);
/// ```
///
/// ```no_run
/// static DEBUG: spin::Once<bool> = spin::Once::new();
/// kernel_param!("debug", DEBUG);
/// ```
#[macro_export]
macro_rules! kernel_param {
    ($name:literal, $slot:path) => {
        ostd::define_kernel_param!($name, $slot, false);
    };
}

/// Declares a kernel command-line parameter that may appear multiple times.
///
/// This is a convenience wrapper around [`ostd::define_kernel_param_vec!`]. It always
/// registers the parameter as a non-early parameter (`early = false`).
///
/// Use this when you want to collect *all* occurrences of the same parameter.
/// Each time the cmdline parser encounters `name`, it will parse the value via
/// `T: FromKernelParam` and `push` it into the vector stored in `slot`.
///
/// # Syntax
/// - `name`: Parameter name string literal (typically ends with `=`).
/// - `slot`: A `'static` storage location of type `Mutex<Vec<T>>`, where `T: FromKernelParam`.
///
/// # Behavior
/// For each occurrence of `name` in the command line, the parsed value is appended
/// to the vector. Invalid values are ignored (i.e. when `T::from_value(...)` returns `None`).
///
/// # Examples
/// ```no_run
/// static CONSOLES: Mutex<Vec<Console>> = Mutex::new(Vec::new());
/// kernel_param_vec!("console=", CONSOLES);
/// ```
///
/// With `console=ttyS0 console=tty0`, the resulting vector will contain two entries
/// in the original order.
#[macro_export]
macro_rules! kernel_param_vec {
    ($name:literal, $slot:path) => {
        ostd::define_kernel_param_vec!($name, $slot, false);
    };
}

#[derive(PartialEq, Debug)]
struct InitprocArgs {
    argv: Vec<CString>,
    envp: Vec<CString>,
}

/// The struct to store the parsed kernel command-line arguments.
#[derive(Debug)]
pub struct KCmdlineArg {
    initproc: InitprocArgs,
    params: Vec<(&'static KernelParam, Option<&'static str>)>,
}

// Define get APIs.
impl KCmdlineArg {
    /// Gets the argument vector (`argv`) of the init process.
    pub fn get_initproc_argv(&self) -> &Vec<CString> {
        &self.initproc.argv
    }

    /// Gets the environment vector (`envp`) of the init process.
    pub fn get_initproc_envp(&self) -> &Vec<CString> {
        &self.initproc.envp
    }
}

// Splits the command line string by spaces but preserve
// ones that are protected by double quotes(`"`).
fn split_arg(input: &str) -> impl Iterator<Item = &str> {
    let mut inside_quotes = false;

    input.split(move |c: char| {
        if c == '"' {
            inside_quotes = !inside_quotes;
        }

        !inside_quotes && c.is_whitespace()
    })
}

// Define the way to parse a string to `KCmdlineArg`.
impl From<&'static str> for KCmdlineArg {
    fn from(cmdline: &'static str) -> Self {
        // What we construct.
        let mut result: KCmdlineArg = KCmdlineArg {
            initproc: InitprocArgs {
                argv: Vec::new(),
                envp: Vec::new(),
            },
            params: Vec::new(),
        };

        // Every thing after the "--" mark is the init arguments.
        let mut kcmdline_end = false;

        // The main parse loop. The processing steps are arranged (not very strictly)
        // by the analysis over the Backus–Naur form syntax tree.
        for arg in split_arg(cmdline) {
            // Cmdline => KernelArg "--" InitArg
            // KernelArg => Arg "\s+" KernelArg | %empty
            // InitArg => Arg "\s+" InitArg | %empty
            if kcmdline_end {
                result.initproc.argv.push(CString::new(arg).unwrap());
                continue;
            }
            if arg == "--" {
                kcmdline_end = true;
                continue;
            }

            // Arg => Entry | Entry "=" Value
            let arg_pattern: Vec<_> = arg.split('=').collect();
            let (entry, value) = match arg_pattern.len() {
                1 => (arg_pattern[0], None),
                2 => (arg_pattern[0], Some(arg_pattern[1])),
                _ => {
                    log::warn!(
                        "[KCmdline] Unable to parse kernel argument {}, skip for now",
                        arg
                    );
                    continue;
                }
            };

            let param = query_kernel_param(entry);

            if let Some(param) = param {
                result.params.push((param, value));
            } else {
                if entry.contains('.') {
                    // The entry contains a dot, which is treated as a module argument.
                    // Unrecognized module arguments are ignored.
                    continue;
                } else if let Some(value) = value {
                    // If the entry is not recognized, it is passed to the init process.
                    // Pattern 'entry=value' is treated as the init environment.
                    let envp_entry = CString::new(entry.to_string() + "=" + value).unwrap();
                    result.initproc.envp.push(envp_entry);
                } else {
                    // If the entry is not recognized, it is passed to the init process.
                    // Pattern 'entry' without value is treated as the init argument.
                    let argv_entry = CString::new(entry.to_string()).unwrap();
                    result.initproc.argv.push(argv_entry);
                }
            }
        }

        result
    }
}

/// The [`KCmdlineArg`] singleton.
pub static KCMDLINE: Once<KCmdlineArg> = Once::new();

#[init_component]
fn init() -> Result<(), ComponentInitError> {
    KCMDLINE.call_once(|| KCmdlineArg::from(ostd::boot::boot_info().kernel_cmdline.as_str()));

    let params = &KCMDLINE.get().unwrap().params;
    let (early, late): (Vec<_>, Vec<_>) =
        params.iter().copied().partition(|(param, _)| param.early());

    early
        .into_iter()
        .for_each(|(param, value)| param.call_setup(value));
    late.into_iter()
        .for_each(|(param, value)| param.call_setup(value));

    Ok(())
}

// All unimplemented parameters should be defined here.
ostd::define_kernel_param_unimpl!("tsc");
ostd::define_kernel_param_unimpl!("no_timer_check");
