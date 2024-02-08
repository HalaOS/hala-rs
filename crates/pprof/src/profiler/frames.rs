use std::{os::raw::c_void, ptr::null_mut};

use crate::c::backtrace_lock;

#[derive(serde::Serialize, serde::Deserialize)]
pub struct Symbol {
    pub name: String,
    pub address: usize,
    pub file_name: String,
    pub line_no: u32,
    pub col_no: u32,
}

/// Call this fn to get callstack, not via [`backtrace::trace`].
///
/// The [``backtrace``] standard api, which uses thread-local keys, may not use in GlobalAlloc.
#[allow(unused)]
pub(super) fn get_backtrace() -> Vec<usize> {
    let mut stack = vec![];

    unsafe {
        let _gurad = backtrace_lock();

        backtrace::trace_unsynchronized(|frame| {
            stack.push(frame.symbol_address() as usize);

            true
        });
    }

    stack
}

/// Call this fn to convert frame symbol address to frame symbol, not via [`backtrace::resolve`]
///
/// The [``backtrace``] standard api, which uses thread-local keys, may not use in GlobalAlloc.
#[allow(unused)]
pub(super) fn frames_to_symbols(frames: &[usize]) -> Vec<Symbol> {
    let mut symbols = vec![];

    let _guard = backtrace_lock();

    for addr in frames {
        let mut proto_symbol = None;

        // get frame symbol object.
        unsafe {
            // Safety: we provide frame to resolve symbol.
            // let _guard = backtrace_lock();

            backtrace::resolve_unsynchronized((*addr) as *mut c_void, |symbol| {
                if proto_symbol.is_none() {
                    proto_symbol = Some(Symbol {
                        name: symbol.name().map(|s| s.to_string()).unwrap_or_default(),
                        address: symbol.addr().unwrap_or(null_mut()) as usize,
                        file_name: symbol
                            .filename()
                            .map(|path| path.to_str().unwrap().to_string())
                            .unwrap_or_default(),
                        line_no: symbol.lineno().unwrap_or_default(),
                        col_no: symbol.colno().unwrap_or_default(),
                    });
                }
            });
        };

        if let Some(symbol) = proto_symbol {
            symbols.push(symbol);
        }
    }

    symbols
}
