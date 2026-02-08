// SPDX-License-Identifier: MPL-2.0

//! Kernel command-line parameter registration and value decoding.
//!
//! This module does **not** parse the full command line string by itself.
//! Instead, it provides the low-level building blocks used by the cmdline
//! parsing component:
//!
//! - A `KernelParam` descriptor type (stored in the `.kernel_param` linker
//!   section).
//! - A `FromKernelParam` trait that all kernel parameter value types must
//!   implement.
//! - Macros (`define_kernel_param!` / `define_kernel_param_vec!`) to register
//!   kernel parameters.
//!
//! At boot, the cmdline component iterates over the registered descriptors,
//! matches them against tokens from the kernel command line, and invokes each
//! descriptor’s setup callback to populate user-provided storage slots
//! (`spin::Once<T>` for single assignment, or `Mutex<Vec<T>>` for collecting
//! repeats).

use spin::Once;

/// The low-level struct to represent a kernel parameter.
///
/// The kernel parameters are defined in the `.kernel_param` section by the
/// `define_kernel_param!` and `define_kernel_param_vec!` macros.
#[repr(C)]
#[derive(Debug)]
pub struct KernelParam {
    name: &'static str,
    setup: fn(Option<&'static str>) -> (),
    early: bool,
    implemented: bool,
}

impl KernelParam {
    /// Creates a new kernel parameter.
    pub const fn new(
        name: &'static str,
        setup: fn(Option<&'static str>) -> (),
        early: bool,
        implemented: bool,
    ) -> Self {
        Self {
            name,
            setup,
            early,
            implemented,
        }
    }

    /// Calls the setup function.
    ///
    /// Invoke the callback which is the internal function defined in the
    /// `define_kernel_param!` and `define_kernel_param_vec!` macros. The
    /// callback will parse the kernel parameter value and store it in the
    /// corresponding slot, i.e. `spin::Once<T>` or `Mutex<Vec<T>>`.
    ///
    /// The `param` argument is the value of the kernel parameter, which is
    /// `None` for parameters without value and `Some(value)` for parameters
    /// with value.
    pub fn call_setup(&self, param: Option<&'static str>) {
        (self.setup)(param);
    }

    /// Gets the name of the kernel parameter.
    pub fn name(&self) -> &str {
        if self.name.ends_with('=') {
            &self.name[..self.name.len() - 1]
        } else {
            self.name
        }
    }

    /// Checks whether the kernel parameter has a value.
    pub fn has_value(&self) -> bool {
        self.name.ends_with('=')
    }

    /// Checks whether the kernel parameter is an early parameter.
    pub fn early(&self) -> bool {
        self.early
    }

    /// Checks whether the kernel parameter is implemented.
    pub fn implemented(&self) -> bool {
        self.implemented
    }
}

fn kernel_params() -> &'static [KernelParam] {
    static PARAMS: Once<&'static [KernelParam]> = Once::new();

    PARAMS.call_once(|| {
        unsafe extern "C" {
            static __kernel_param_start: u8;
            static __kernel_param_end: u8;
        }

        // SAFETY: The range is guaranteed to be valid as it is defined in the
        // `.kernel_param` section.
        unsafe {
            let start = core::ptr::addr_of!(__kernel_param_start) as *const KernelParam;
            let end = core::ptr::addr_of!(__kernel_param_end) as *const KernelParam;
            let len = end.offset_from(start) as usize;
            core::slice::from_raw_parts(start, len)
        }
    })
}

/// Queries a kernel parameter by name.
pub fn query_kernel_param(name: &str) -> Option<&KernelParam> {
    for kp in kernel_params() {
        if kp.name() == name {
            if !kp.implemented() {
                crate::early_println!("Warning: kernel parameter `{}` is unimplemented", name);
            }
            return Some(kp);
        }
    }
    None
}

/// A trait to convert a kernel parameter value to a Rust type.
pub trait FromKernelParam: Sized + 'static {
    /// Converts a kernel parameter value into a Rust type.
    ///
    /// # Input
    /// - `None`: the parameter was present without an explicit value
    ///   (flag-style), e.g. `debug`.
    /// - `Some(value)`: the parameter was provided with a value, e.g.
    ///   `log_level=info` where `value` is `"info"`.
    ///
    /// # Return value
    /// Implementations should return:
    /// - `Some(Self)` if the input is acceptable and can be constructed.
    /// - `None` if the input is invalid, or if a required value is missing.
    fn from_value(value: Option<&'static str>) -> Option<Self>;
}

/// A marker type for kernel parameters without value.
///
/// Recommends all kernel parameters without value to use this type as the slot
/// type.
pub struct KernelFlag;

impl FromKernelParam for KernelFlag {
    fn from_value(value: Option<&'static str>) -> Option<Self> {
        if value.is_none() { Some(Self) } else { None }
    }
}

impl<T> FromKernelParam for T
where
    T: core::str::FromStr + 'static,
{
    fn from_value(value: Option<&'static str>) -> Option<Self> {
        let s = value?;
        s.trim().parse::<T>().ok()
    }
}

#[macro_export]
#[doc(hidden)]
macro_rules! __define_kernel_param_common {
    ($name:literal, $early:expr, $setup_fn:ident, $slot:expr, $implemented:expr) => {
        use $crate::boot::cmdline::KernelParam;

        #[used]
        // SAFETY: This is properly handled in the linker script.
        #[unsafe(link_section = ".kernel_param")]
        static __KERNEL_PARAM: KernelParam = KernelParam::new(
            $name,
            |value| $setup_fn(value, &$slot),
            $early,
            $implemented,
        );
    };
}

/// Defines a kernel command-line parameter and registers it into the
/// `.kernel_param` linker section.
///
/// # Syntax
/// - `name`: Parameter name string literal. Use a trailing `=` for parameters
///    that take a value, e.g. `"foo="` for `foo=123`. Omit `=` for flag-style
///    parameters without a value, e.g. `"bar"` for `bar`.
/// - `slot`: A `'static` storage location of type `spin::Once<T>`.
/// - `early`: Whether this parameter is an *early* parameter (parsed before
///    non-early ones).
///
/// # Behavior
/// When the command line parser encounters `name`, it will call the registered
/// setup callback, parse the optional value via `T: FromKernelParam`, and
/// store the result into `slot`. Since the slot is `spin::Once<T>`, only the
/// first successfully parsed value is recorded.
///
/// # Examples
/// - Value parameter:
///   `define_kernel_param!("foo=", FOO, false)` matches `foo=...` and stores
///   into `FOO`.
/// - Flag parameter:
///   `define_kernel_param!("bar", BAR, false)` matches `bar` (no value) and
///   stores into `BAR`.
///
/// # Note on initialization order
/// Prefer not to use this macro directly. Components that declare parameters
/// may be initialized earlier than the component that parses the command line,
/// leading to initialization-order issues. Use the wrapper macros provided by
/// the cmdline parsing component (e.g. `aster-cmdline`) so component
/// dependencies enforce the correct order.
#[macro_export]
macro_rules! define_kernel_param {
    ($name:literal, $slot:path, $early:expr) => {
        const _: () = {
            use $crate::boot::cmdline::FromKernelParam;

            fn __setup<T: FromKernelParam + 'static>(
                value: Option<&'static str>,
                slot: &'static spin::Once<T>,
            ) {
                if let Some(v) = T::from_value(value) {
                    let _ = slot.call_once(|| v);
                }
            }

            $crate::__define_kernel_param_common!($name, $early, __setup, $slot, true);
        };
    };
}

/// Defines a kernel parameter that can have multiple values.
///
/// This macro is almost same as `define_kernel_param`, but the `slot` is a
/// `Mutex<Vec<T>>` that holds multiple values of the kernel parameter. The
/// setup function will push the parsed value to the vector.
#[macro_export]
macro_rules! define_kernel_param_vec {
    ($name:literal, $slot:path, $early:expr) => {
        const _: () = {
            use $crate::boot::cmdline::FromKernelParam;

            fn __setup_vec<T: FromKernelParam + 'static>(
                value: Option<&'static str>,
                slot: &'static $crate::sync::Mutex<Vec<T>>,
            ) {
                if let Some(v) = T::from_value(value) {
                    slot.lock().push(v);
                }
            }

            $crate::__define_kernel_param_common!($name, $early, __setup_vec, $slot, true);
        };
    };
}

/// Defines a kernel parameter that is not implemented.
///
/// This is useful while Asterinas is under active development: some Linux
/// kernel parameters are commonly present in boot configurations, but
/// Asterinas may not support them yet.
///
/// Defining such parameters with this macro has two effects:
/// - The cmdline parser will treat the parameter as *known*, so it will not be
///   forwarded to the init process.
/// - The parameter will be reported as unimplemented (via
///   `KernelParam::implemented()`), allowing the parser to emit a warning if
///   desired.
///
/// This matches Linux behavior: many kernel-only parameters are consumed by
/// the kernel and are not passed through to init process.
///
/// # Syntax
/// - `name`: Parameter name string literal (with or without a trailing `=`).
#[macro_export]
macro_rules! define_kernel_param_unimpl {
    ($name:literal) => {
        const _: () = {
            fn __setup(_value: Option<&'static str>, _slot: &'static ()) {
                // Do nothing since this parameter is unimplemented.
            }

            $crate::__define_kernel_param_common!($name, false, __setup, (), false);
        };
    };
}
