use std::{
    ffi::c_void,
    fmt,
    path::{Path, PathBuf},
};

use backtrace::{
    resolve_frame_unsynchronized, resolve_unsynchronized, trace_unsynchronized, BytesOrWideString,
    Symbol, SymbolName,
};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::external::backtrace_lock;

const HEX_WIDTH: usize = 2 + 2 * core::mem::size_of::<usize>();

/// A formatter for backtraces.
///
/// This type can be used to print a backtrace regardless of where the backtrace
/// itself comes from. If you have a `Backtrace` type then its `Debug`
/// implementation already uses this printing format.
pub struct BacktraceFmt<'a, 'b> {
    fmt: &'a mut fmt::Formatter<'b>,
    frame_index: usize,
    format: PrintFmt,
    print_path:
        &'a mut (dyn FnMut(&mut fmt::Formatter<'_>, BytesOrWideString<'_>) -> fmt::Result + 'b),
}

/// The styles of printing that we can print
#[derive(Copy, Clone, Eq, PartialEq)]
pub enum PrintFmt {
    /// Prints a terser backtrace which ideally only contains relevant information
    Short,
    /// Prints a backtrace that contains all possible information
    Full,
    #[doc(hidden)]
    __Nonexhaustive,
}

impl<'a, 'b> BacktraceFmt<'a, 'b> {
    /// Create a new `BacktraceFmt` which will write output to the provided
    /// `fmt`.
    ///
    /// The `format` argument will control the style in which the backtrace is
    /// printed, and the `print_path` argument will be used to print the
    /// `BytesOrWideString` instances of filenames. This type itself doesn't do
    /// any printing of filenames, but this callback is required to do so.
    pub fn new(
        fmt: &'a mut fmt::Formatter<'b>,
        format: PrintFmt,
        print_path: &'a mut (dyn FnMut(&mut fmt::Formatter<'_>, BytesOrWideString<'_>) -> fmt::Result
                     + 'b),
    ) -> Self {
        BacktraceFmt {
            fmt,
            frame_index: 0,
            format,
            print_path,
        }
    }

    /// Prints a preamble for the backtrace about to be printed.
    ///
    /// This is required on some platforms for backtraces to be fully
    /// symbolicated later, and otherwise this should just be the first method
    /// you call after creating a `BacktraceFmt`.
    pub fn add_context(&mut self) -> fmt::Result {
        #[cfg(target_os = "fuchsia")]
        fuchsia::print_dso_context(self.fmt)?;
        Ok(())
    }

    /// Adds a frame to the backtrace output.
    ///
    /// This commit returns an RAII instance of a `BacktraceFrameFmt` which can be used
    /// to actually print a frame, and on destruction it will increment the
    /// frame counter.
    pub fn frame(&mut self) -> BacktraceFrameFmt<'_, 'a, 'b> {
        BacktraceFrameFmt {
            fmt: self,
            symbol_index: 0,
        }
    }

    /// Completes the backtrace output.
    ///
    /// This is currently a no-op but is added for future compatibility with
    /// backtrace formats.
    pub fn finish(&mut self) -> fmt::Result {
        #[cfg(target_os = "fuchsia")]
        fuchsia::finish_context(self.fmt)?;
        Ok(())
    }

    /// Inserts a message in the backtrace output.
    ///
    /// This allows information to be inserted between frames,
    /// and won't increment the `frame_index` unlike the `frame`
    /// method.
    pub fn message(&mut self, msg: &str) -> fmt::Result {
        self.fmt.write_str(msg)
    }

    /// Return the inner formatter.
    ///
    /// This is used for writing custom information between frames with `write!` and `writeln!`,
    /// and won't increment the `frame_index` unlike the `frame` method.
    pub fn formatter(&mut self) -> &mut fmt::Formatter<'b> {
        self.fmt
    }
}

/// A formatter for just one frame of a backtrace.
///
/// This type is created by the `BacktraceFmt::frame` function.
pub struct BacktraceFrameFmt<'fmt, 'a, 'b> {
    fmt: &'fmt mut BacktraceFmt<'a, 'b>,
    symbol_index: usize,
}

impl BacktraceFrameFmt<'_, '_, '_> {
    /// Prints a `BacktraceFrame` with this frame formatter.
    ///
    /// This will recursively print all `BacktraceSymbol` instances within the
    /// `BacktraceFrame`.
    ///
    /// # Required features
    ///
    /// This function requires the `std` feature of the `backtrace` crate to be
    /// enabled, and the `std` feature is enabled by default.
    pub fn backtrace_frame(&mut self, frame: &BacktraceFrame) -> fmt::Result {
        let symbols = frame.symbols();
        for symbol in symbols {
            self.backtrace_symbol(frame, symbol)?;
        }
        if symbols.is_empty() {
            self.print_raw(frame.ip(), None, None, None)?;
        }
        Ok(())
    }

    /// Prints a `BacktraceSymbol` within a `BacktraceFrame`.
    ///
    /// # Required features
    ///
    /// This function requires the `std` feature of the `backtrace` crate to be
    /// enabled, and the `std` feature is enabled by default.
    pub fn backtrace_symbol(
        &mut self,
        frame: &BacktraceFrame,
        symbol: &BacktraceSymbol,
    ) -> fmt::Result {
        self.print_raw_with_column(
            frame.ip(),
            symbol.name(),
            // TODO: this isn't great that we don't end up printing anything
            // with non-utf8 filenames. Thankfully almost everything is utf8 so
            // this shouldn't be too bad.
            symbol
                .filename()
                .and_then(|p| Some(BytesOrWideString::Bytes(p.to_str()?.as_bytes()))),
            symbol.lineno(),
            symbol.colno(),
        )?;
        Ok(())
    }

    /// Prints a raw traced `Frame` and `Symbol`, typically from within the raw
    /// callbacks of this crate.
    pub fn symbol(&mut self, frame: &backtrace::Frame, symbol: &backtrace::Symbol) -> fmt::Result {
        self.print_raw_with_column(
            frame.ip(),
            symbol.name(),
            symbol.filename_raw(),
            symbol.lineno(),
            symbol.colno(),
        )?;
        Ok(())
    }

    /// Adds a raw frame to the backtrace output.
    ///
    /// This method, unlike the previous, takes the raw arguments in case
    /// they're being source from different locations. Note that this may be
    /// called multiple times for one frame.
    pub fn print_raw(
        &mut self,
        frame_ip: *mut c_void,
        symbol_name: Option<SymbolName<'_>>,
        filename: Option<BytesOrWideString<'_>>,
        lineno: Option<u32>,
    ) -> fmt::Result {
        self.print_raw_with_column(frame_ip, symbol_name, filename, lineno, None)
    }

    /// Adds a raw frame to the backtrace output, including column information.
    ///
    /// This method, like the previous, takes the raw arguments in case
    /// they're being source from different locations. Note that this may be
    /// called multiple times for one frame.
    pub fn print_raw_with_column(
        &mut self,
        frame_ip: *mut c_void,
        symbol_name: Option<SymbolName<'_>>,
        filename: Option<BytesOrWideString<'_>>,
        lineno: Option<u32>,
        colno: Option<u32>,
    ) -> fmt::Result {
        // Fuchsia is unable to symbolize within a process so it has a special
        // format which can be used to symbolize later. Print that instead of
        // printing addresses in our own format here.
        if cfg!(target_os = "fuchsia") {
            self.print_raw_fuchsia(frame_ip)?;
        } else {
            self.print_raw_generic(frame_ip, symbol_name, filename, lineno, colno)?;
        }
        self.symbol_index += 1;
        Ok(())
    }

    #[allow(unused_mut)]
    fn print_raw_generic(
        &mut self,
        mut frame_ip: *mut c_void,
        symbol_name: Option<SymbolName<'_>>,
        filename: Option<BytesOrWideString<'_>>,
        lineno: Option<u32>,
        colno: Option<u32>,
    ) -> fmt::Result {
        // No need to print "null" frames, it basically just means that the
        // system backtrace was a bit eager to trace back super far.
        if let PrintFmt::Short = self.fmt.format {
            if frame_ip.is_null() {
                return Ok(());
            }
        }

        // To reduce TCB size in Sgx enclave, we do not want to implement symbol
        // resolution functionality.  Rather, we can print the offset of the
        // address here, which could be later mapped to correct function.
        #[cfg(all(feature = "std", target_env = "sgx", target_vendor = "fortanix"))]
        {
            let image_base = std::os::fortanix_sgx::mem::image_base();
            frame_ip = usize::wrapping_sub(frame_ip as usize, image_base as _) as _;
        }

        // Print the index of the frame as well as the optional instruction
        // pointer of the frame. If we're beyond the first symbol of this frame
        // though we just print appropriate whitespace.
        if self.symbol_index == 0 {
            write!(self.fmt.fmt, "{:4}: ", self.fmt.frame_index)?;
            if let PrintFmt::Full = self.fmt.format {
                write!(self.fmt.fmt, "{:1$?} - ", frame_ip, HEX_WIDTH)?;
            }
        } else {
            write!(self.fmt.fmt, "      ")?;
            if let PrintFmt::Full = self.fmt.format {
                write!(self.fmt.fmt, "{:1$}", "", HEX_WIDTH + 3)?;
            }
        }

        // Next up write out the symbol name, using the alternate formatting for
        // more information if we're a full backtrace. Here we also handle
        // symbols which don't have a name,
        match (symbol_name, &self.fmt.format) {
            (Some(name), PrintFmt::Short) => write!(self.fmt.fmt, "{:#}", name)?,
            (Some(name), PrintFmt::Full) => write!(self.fmt.fmt, "{}", name)?,
            (None, _) | (_, PrintFmt::__Nonexhaustive) => write!(self.fmt.fmt, "<unknown>")?,
        }
        self.fmt.fmt.write_str("\n")?;

        // And last up, print out the filename/line number if they're available.
        if let (Some(file), Some(line)) = (filename, lineno) {
            self.print_fileline(file, line, colno)?;
        }

        Ok(())
    }

    fn print_fileline(
        &mut self,
        file: BytesOrWideString<'_>,
        line: u32,
        colno: Option<u32>,
    ) -> fmt::Result {
        // Filename/line are printed on lines under the symbol name, so print
        // some appropriate whitespace to sort of right-align ourselves.
        if let PrintFmt::Full = self.fmt.format {
            write!(self.fmt.fmt, "{:1$}", "", HEX_WIDTH)?;
        }
        write!(self.fmt.fmt, "             at ")?;

        // Delegate to our internal callback to print the filename and then
        // print out the line number.
        (self.fmt.print_path)(self.fmt.fmt, file)?;
        write!(self.fmt.fmt, ":{}", line)?;

        // Add column number, if available.
        if let Some(colno) = colno {
            write!(self.fmt.fmt, ":{}", colno)?;
        }

        write!(self.fmt.fmt, "\n")?;
        Ok(())
    }

    fn print_raw_fuchsia(&mut self, frame_ip: *mut c_void) -> fmt::Result {
        // We only care about the first symbol of a frame
        if self.symbol_index == 0 {
            self.fmt.fmt.write_str("{{{bt:")?;
            write!(self.fmt.fmt, "{}:{:?}", self.fmt.frame_index, frame_ip)?;
            self.fmt.fmt.write_str("}}}\n")?;
        }
        Ok(())
    }
}

impl Drop for BacktraceFrameFmt<'_, '_, '_> {
    fn drop(&mut self) {
        self.fmt.frame_index += 1;
    }
}

/// Representation of an owned and self-contained backtrace.
///
/// This structure can be used to capture a backtrace at various points in a
/// program and later used to inspect what the backtrace was at that time.
///
/// `Backtrace` supports pretty-printing of backtraces through its `Debug`
/// implementation.
///
/// # Required features
///
/// This function requires the `std` feature of the `backtrace` crate to be
/// enabled, and the `std` feature is enabled by default.
#[derive(Clone)]
#[cfg_attr(feature = "serialize-rustc", derive(RustcDecodable, RustcEncodable))]
#[cfg_attr(feature = "serde", derive(Deserialize, Serialize))]
pub struct Backtrace {
    // Frames here are listed from top-to-bottom of the stack
    frames: Vec<BacktraceFrame>,
    // The index we believe is the actual start of the backtrace, omitting
    // frames like `Backtrace::new` and `backtrace::trace`.
    actual_start_index: usize,
}

fn _assert_send_sync() {
    fn _assert<T: Send + Sync>() {}
    _assert::<Backtrace>();
}

/// Captured version of a frame in a backtrace.
///
/// This type is returned as a list from `Backtrace::frames` and represents one
/// stack frame in a captured backtrace.
///
/// # Required features
///
/// This function requires the `std` feature of the `backtrace` crate to be
/// enabled, and the `std` feature is enabled by default.
#[derive(Clone)]
pub struct BacktraceFrame {
    pub(crate) frame: Frame,
    pub(crate) symbols: Option<Vec<BacktraceSymbol>>,
}

#[derive(Clone)]
pub(crate) enum Frame {
    Raw(backtrace::Frame),
    #[allow(dead_code)]
    Deserialized {
        ip: usize,
        symbol_address: usize,
        module_base_address: Option<usize>,
    },
}

impl Frame {
    fn ip(&self) -> *mut c_void {
        match *self {
            Frame::Raw(ref f) => f.ip(),
            Frame::Deserialized { ip, .. } => ip as *mut c_void,
        }
    }

    fn symbol_address(&self) -> *mut c_void {
        match *self {
            Frame::Raw(ref f) => f.symbol_address(),
            Frame::Deserialized { symbol_address, .. } => symbol_address as *mut c_void,
        }
    }

    fn module_base_address(&self) -> Option<*mut c_void> {
        match *self {
            Frame::Raw(ref f) => f.module_base_address(),
            Frame::Deserialized {
                module_base_address,
                ..
            } => module_base_address.map(|addr| addr as *mut c_void),
        }
    }
}

/// Captured version of a symbol in a backtrace.
///
/// This type is returned as a list from `BacktraceFrame::symbols` and
/// represents the metadata for a symbol in a backtrace.
///
/// # Required features
///
/// This function requires the `std` feature of the `backtrace` crate to be
/// enabled, and the `std` feature is enabled by default.
#[derive(Clone)]
#[cfg_attr(feature = "serialize-rustc", derive(RustcDecodable, RustcEncodable))]
#[cfg_attr(feature = "serde", derive(Deserialize, Serialize))]
pub struct BacktraceSymbol {
    pub(crate) name: Option<Vec<u8>>,
    pub(crate) addr: Option<usize>,
    pub(crate) filename: Option<PathBuf>,
    pub(crate) lineno: Option<u32>,
    pub(crate) colno: Option<u32>,
}

impl Backtrace {
    /// Captures a backtrace at the callsite of this function, returning an
    /// owned representation.
    ///
    /// This function is useful for representing a backtrace as an object in
    /// Rust. This returned value can be sent across threads and printed
    /// elsewhere, and the purpose of this value is to be entirely self
    /// contained.
    ///
    /// Note that on some platforms acquiring a full backtrace and resolving it
    /// can be extremely expensive. If the cost is too much for your application
    /// it's recommended to instead use `Backtrace::new_unresolved()` which
    /// avoids the symbol resolution step (which typically takes the longest)
    /// and allows deferring that to a later date.
    ///
    /// # Examples
    ///
    /// ```
    /// use backtrace::Backtrace;
    ///
    /// let current_backtrace = Backtrace::new();
    /// ```
    ///
    /// # Required features
    ///
    /// This function requires the `std` feature of the `backtrace` crate to be
    /// enabled, and the `std` feature is enabled by default.
    #[inline(never)] // want to make sure there's a frame here to remove
    pub fn new() -> Backtrace {
        let mut bt = Self::create(Self::new as usize);
        bt.resolve();
        bt
    }

    /// Similar to `new` except that this does not resolve any symbols, this
    /// simply captures the backtrace as a list of addresses.
    ///
    /// At a later time the `resolve` function can be called to resolve this
    /// backtrace's symbols into readable names. This function exists because
    /// the resolution process can sometimes take a significant amount of time
    /// whereas any one backtrace may only be rarely printed.
    ///
    /// # Examples
    ///
    /// ```
    /// use backtrace::Backtrace;
    ///
    /// let mut current_backtrace = Backtrace::new_unresolved();
    /// println!("{:?}", current_backtrace); // no symbol names
    /// current_backtrace.resolve();
    /// println!("{:?}", current_backtrace); // symbol names now present
    /// ```
    ///
    /// # Required features
    ///
    /// This function requires the `std` feature of the `backtrace` crate to be
    /// enabled, and the `std` feature is enabled by default.
    #[inline(never)] // want to make sure there's a frame here to remove
    pub fn new_unresolved() -> Backtrace {
        Self::create(Self::new_unresolved as usize)
    }

    fn create(ip: usize) -> Backtrace {
        let mut frames = Vec::new();
        let mut actual_start_index = None;
        let _guard = backtrace_lock();

        unsafe {
            trace_unsynchronized(|frame| {
                frames.push(BacktraceFrame {
                    frame: Frame::Raw(frame.clone()),
                    symbols: None,
                });

                if frame.symbol_address() as usize == ip && actual_start_index.is_none() {
                    actual_start_index = Some(frames.len());
                }
                true
            });
        }

        Backtrace {
            frames,
            actual_start_index: actual_start_index.unwrap_or(0),
        }
    }

    /// Returns the frames from when this backtrace was captured.
    ///
    /// The first entry of this slice is likely the function `Backtrace::new`,
    /// and the last frame is likely something about how this thread or the main
    /// function started.
    ///
    /// # Required features
    ///
    /// This function requires the `std` feature of the `backtrace` crate to be
    /// enabled, and the `std` feature is enabled by default.
    pub fn frames(&self) -> &[BacktraceFrame] {
        &self.frames[self.actual_start_index..]
    }

    /// If this backtrace was created from `new_unresolved` then this function
    /// will resolve all addresses in the backtrace to their symbolic names.
    ///
    /// If this backtrace has been previously resolved or was created through
    /// `new`, this function does nothing.
    ///
    /// # Required features
    ///
    /// This function requires the `std` feature of the `backtrace` crate to be
    /// enabled, and the `std` feature is enabled by default.
    pub fn resolve(&mut self) {
        for frame in self.frames.iter_mut().filter(|f| f.symbols.is_none()) {
            let mut symbols = Vec::new();
            {
                let sym = |symbol: &Symbol| {
                    symbols.push(BacktraceSymbol {
                        name: symbol.name().map(|m| m.as_bytes().to_vec()),
                        addr: symbol.addr().map(|a| a as usize),
                        filename: symbol.filename().map(|m| m.to_owned()),
                        lineno: symbol.lineno(),
                        colno: symbol.colno(),
                    });
                };
                match frame.frame {
                    Frame::Raw(ref f) => {
                        let _guard = backtrace_lock();
                        unsafe { resolve_frame_unsynchronized(f, sym) }
                    }
                    Frame::Deserialized { ip, .. } => {
                        let _guard = backtrace_lock();
                        unsafe { resolve_unsynchronized(ip as *mut c_void, sym) };
                    }
                }
            }
            frame.symbols = Some(symbols);
        }
    }
}

impl From<Vec<BacktraceFrame>> for Backtrace {
    fn from(frames: Vec<BacktraceFrame>) -> Self {
        Backtrace {
            frames,
            actual_start_index: 0,
        }
    }
}

impl From<backtrace::Frame> for BacktraceFrame {
    fn from(frame: backtrace::Frame) -> BacktraceFrame {
        BacktraceFrame {
            frame: Frame::Raw(frame),
            symbols: None,
        }
    }
}

impl Into<Vec<BacktraceFrame>> for Backtrace {
    fn into(self) -> Vec<BacktraceFrame> {
        self.frames
    }
}

impl BacktraceFrame {
    /// Same as `Frame::ip`
    ///
    /// # Required features
    ///
    /// This function requires the `std` feature of the `backtrace` crate to be
    /// enabled, and the `std` feature is enabled by default.
    pub fn ip(&self) -> *mut c_void {
        self.frame.ip() as *mut c_void
    }

    /// Same as `Frame::symbol_address`
    ///
    /// # Required features
    ///
    /// This function requires the `std` feature of the `backtrace` crate to be
    /// enabled, and the `std` feature is enabled by default.
    pub fn symbol_address(&self) -> *mut c_void {
        self.frame.symbol_address() as *mut c_void
    }

    /// Same as `Frame::module_base_address`
    ///
    /// # Required features
    ///
    /// This function requires the `std` feature of the `backtrace` crate to be
    /// enabled, and the `std` feature is enabled by default.
    pub fn module_base_address(&self) -> Option<*mut c_void> {
        self.frame
            .module_base_address()
            .map(|addr| addr as *mut c_void)
    }

    /// Returns the list of symbols that this frame corresponds to.
    ///
    /// Normally there is only one symbol per frame, but sometimes if a number
    /// of functions are inlined into one frame then multiple symbols will be
    /// returned. The first symbol listed is the "innermost function", whereas
    /// the last symbol is the outermost (last caller).
    ///
    /// Note that if this frame came from an unresolved backtrace then this will
    /// return an empty list.
    ///
    /// # Required features
    ///
    /// This function requires the `std` feature of the `backtrace` crate to be
    /// enabled, and the `std` feature is enabled by default.
    pub fn symbols(&self) -> &[BacktraceSymbol] {
        self.symbols.as_ref().map(|s| &s[..]).unwrap_or(&[])
    }
}

impl BacktraceSymbol {
    /// Same as `Symbol::name`
    ///
    /// # Required features
    ///
    /// This function requires the `std` feature of the `backtrace` crate to be
    /// enabled, and the `std` feature is enabled by default.
    pub fn name(&self) -> Option<SymbolName<'_>> {
        self.name.as_ref().map(|s| SymbolName::new(s))
    }

    /// Same as `Symbol::addr`
    ///
    /// # Required features
    ///
    /// This function requires the `std` feature of the `backtrace` crate to be
    /// enabled, and the `std` feature is enabled by default.
    pub fn addr(&self) -> Option<*mut c_void> {
        self.addr.map(|s| s as *mut c_void)
    }

    /// Same as `Symbol::filename`
    ///
    /// # Required features
    ///
    /// This function requires the `std` feature of the `backtrace` crate to be
    /// enabled, and the `std` feature is enabled by default.
    pub fn filename(&self) -> Option<&Path> {
        self.filename.as_ref().map(|p| &**p)
    }

    /// Same as `Symbol::lineno`
    ///
    /// # Required features
    ///
    /// This function requires the `std` feature of the `backtrace` crate to be
    /// enabled, and the `std` feature is enabled by default.
    pub fn lineno(&self) -> Option<u32> {
        self.lineno
    }

    /// Same as `Symbol::colno`
    ///
    /// # Required features
    ///
    /// This function requires the `std` feature of the `backtrace` crate to be
    /// enabled, and the `std` feature is enabled by default.
    pub fn colno(&self) -> Option<u32> {
        self.colno
    }
}

impl fmt::Debug for Backtrace {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        let full = fmt.alternate();
        let (frames, style) = if full {
            (&self.frames[..], PrintFmt::Full)
        } else {
            (&self.frames[self.actual_start_index..], PrintFmt::Short)
        };

        // When printing paths we try to strip the cwd if it exists, otherwise
        // we just print the path as-is. Note that we also only do this for the
        // short format, because if it's full we presumably want to print
        // everything.
        let cwd = std::env::current_dir();
        let mut print_path = move |fmt: &mut fmt::Formatter<'_>, path: BytesOrWideString<'_>| {
            let path = path.into_path_buf();
            if !full {
                if let Ok(cwd) = &cwd {
                    if let Ok(suffix) = path.strip_prefix(cwd) {
                        return fmt::Display::fmt(&suffix.display(), fmt);
                    }
                }
            }
            fmt::Display::fmt(&path.display(), fmt)
        };

        let mut f = BacktraceFmt::new(fmt, style, &mut print_path);
        f.add_context()?;
        for frame in frames {
            f.frame().backtrace_frame(frame)?;
        }
        f.finish()?;
        Ok(())
    }
}

impl Default for Backtrace {
    fn default() -> Backtrace {
        Backtrace::new()
    }
}

impl fmt::Debug for BacktraceFrame {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt.debug_struct("BacktraceFrame")
            .field("ip", &self.ip())
            .field("symbol_address", &self.symbol_address())
            .finish()
    }
}

impl fmt::Debug for BacktraceSymbol {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt.debug_struct("BacktraceSymbol")
            .field("name", &self.name())
            .field("addr", &self.addr())
            .field("filename", &self.filename())
            .field("lineno", &self.lineno())
            .field("colno", &self.colno())
            .finish()
    }
}

#[cfg(feature = "serialize-rustc")]
mod rustc_serialize_impls {
    use super::*;
    use rustc_serialize::{Decodable, Decoder, Encodable, Encoder};

    #[derive(RustcEncodable, RustcDecodable)]
    struct SerializedFrame {
        ip: usize,
        symbol_address: usize,
        module_base_address: Option<usize>,
        symbols: Option<Vec<BacktraceSymbol>>,
    }

    impl Decodable for BacktraceFrame {
        fn decode<D>(d: &mut D) -> Result<Self, D::Error>
        where
            D: Decoder,
        {
            let frame: SerializedFrame = SerializedFrame::decode(d)?;
            Ok(BacktraceFrame {
                frame: Frame::Deserialized {
                    ip: frame.ip,
                    symbol_address: frame.symbol_address,
                    module_base_address: frame.module_base_address,
                },
                symbols: frame.symbols,
            })
        }
    }

    impl Encodable for BacktraceFrame {
        fn encode<E>(&self, e: &mut E) -> Result<(), E::Error>
        where
            E: Encoder,
        {
            let BacktraceFrame { frame, symbols } = self;
            SerializedFrame {
                ip: frame.ip() as usize,
                symbol_address: frame.symbol_address() as usize,
                module_base_address: frame.module_base_address().map(|addr| addr as usize),
                symbols: symbols.clone(),
            }
            .encode(e)
        }
    }
}

#[cfg(feature = "serde")]
mod serde_impls {
    use super::*;
    use serde::de::Deserializer;
    use serde::ser::Serializer;
    use serde::{Deserialize, Serialize};

    #[derive(Serialize, Deserialize)]
    struct SerializedFrame {
        ip: usize,
        symbol_address: usize,
        module_base_address: Option<usize>,
        symbols: Option<Vec<BacktraceSymbol>>,
    }

    impl Serialize for BacktraceFrame {
        fn serialize<S>(&self, s: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            let BacktraceFrame { frame, symbols } = self;
            SerializedFrame {
                ip: frame.ip() as usize,
                symbol_address: frame.symbol_address() as usize,
                module_base_address: frame.module_base_address().map(|addr| addr as usize),
                symbols: symbols.clone(),
            }
            .serialize(s)
        }
    }

    impl<'a> Deserialize<'a> for BacktraceFrame {
        fn deserialize<D>(d: D) -> Result<Self, D::Error>
        where
            D: Deserializer<'a>,
        {
            let frame: SerializedFrame = SerializedFrame::deserialize(d)?;
            Ok(BacktraceFrame {
                frame: Frame::Deserialized {
                    ip: frame.ip,
                    symbol_address: frame.symbol_address,
                    module_base_address: frame.module_base_address,
                },
                symbols: frame.symbols,
            })
        }
    }
}
