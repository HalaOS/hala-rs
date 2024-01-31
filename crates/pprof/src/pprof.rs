use crate::alloc::HeapProfilingWriter;

/// a [`HeapProfilingWriter`] implementation that converts sample data to google perftools format.
pub struct HeapProfilingPerfToolsBuilder {}

impl HeapProfilingWriter for HeapProfilingPerfToolsBuilder {
    fn write_block(&mut self, _block: *mut u8, _bt: &crate::backtrace::Backtrace) {
        todo!()
    }
}
